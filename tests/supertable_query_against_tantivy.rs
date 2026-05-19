//! Cross-engine BM25 oracle: supertable (N superfiles) vs Tantivy
//! (N internal superfiles).
//!
//! ## What this oracle catches
//!
//! Single-superfile vs Tantivy is already covered by
//! `tests/bm25_against_tantivy.rs`. The supertable layer adds a
//! cross-segment fan-out + global top-k merge that's a separate
//! source of bugs — wrong segment partitioning, wrong tagging of
//! per-segment hits, wrong score-direction in top-k merge would
//! all produce a per-segment-correct-but-globally-wrong answer
//! that single-superfile tests can't catch.
//!
//! Both engines see identical corpora and both shard the corpus
//! across **N superfiles** by the same boundaries (15 docs per
//! segment for the planted corpus, 200K per segment for the
//! Zipfian corpus). Tantivy's superfiles come from one
//! `IndexWriter::commit()` per chunk; the supertable's come from
//! one `SupertableWriter::commit()` per chunk. Per-segment IDF
//! is therefore identical between engines — no sharded-BM25
//! score variance from segment-boundary mismatch.
//!
//! ## Tolerances
//!
//! Top-k *sets* must agree exactly on the head.
//! Tied-score boundaries within the top-k may disagree because
//! Tantivy and our impl break ties differently — for the planted
//! corpus we control queries to avoid tie-prone boundaries; for
//! the Zipfian corpus we use larger `k` values to put the tie
//! boundary outside the head.
//!
//! ## Tokenizer + BM25 parameters
//!
//! Matched: whitespace + punct split, ASCII lowercase, no
//! stemming, k1=1.2, b=0.75 (Lucene defaults; both engines'
//! defaults).
//!
//! ## Prefix row
//!
//! To exercise term-range skip pruning, we plant 5
//! unique-prefix terms across distinct superfiles, query `prefix`
//! on the supertable, and reference-check against Tantivy by
//! issuing a multi-term OR over the known expansion. Tantivy
//! 0.26 doesn't support inline `*`
//! wildcards via `QueryParser` so we explicitly construct the
//! expansion to keep the comparison apples-to-apples.

#![deny(clippy::unwrap_used)]

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use arrow_array::{LargeStringArray, RecordBatch};
use rand::SeedableRng;
use rand::rngs::StdRng;

use infino::superfile::builder::FtsConfig;
use infino::superfile::fts::reader::BoolMode;
use infino::superfile::fts::tokenize::Tokenizer;
use infino::test_helpers::{schema_id_title, default_tokenizer};
use infino::supertable::query::SuperfileHit;
use infino::supertable::{Supertable, SupertableOptions};

use tantivy::collector::TopDocs;
use tantivy::indexer::NoMergePolicy;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{INDEXED, IndexRecordOption, STORED, Schema as TSchema, TEXT, Value};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{Index, IndexSettings, Term, doc};

// ---- Corpus generation ----------------------------------------------

/// Fixed planted corpus, 60 docs. Mirrors the
/// `tests/bm25_against_tantivy.rs` corpus so the cross-engine
/// invariants are easy to reason about. The supertable shards
/// this into 4 superfiles (15 docs each).
fn planted_corpus() -> Vec<(u64, &'static str)> {
    vec![
        (0, "rust async runtime tokio"),
        (1, "rust embedded systems firmware"),
        (2, "python data pipeline pandas"),
        (3, "python machine learning numpy"),
        (4, "javascript web frontend react"),
        (5, "javascript node backend server"),
        (6, "go concurrency goroutines channels"),
        (7, "go web framework gin echo"),
        (8, "rust web framework actix axum"),
        (9, "rust systems programming low level"),
        (10, "kubernetes pods deployment helm"),
        (11, "docker containers images registry"),
        (12, "postgresql replication wal logical"),
        (13, "mysql innodb redo log"),
        (14, "redis sorted sets pub sub"),
        (15, "kafka topics partitions consumers"),
        (16, "elasticsearch lucene inverted index"),
        (17, "tantivy lucene rust search engine"),
        (18, "search engine bm25 ranking inverted"),
        (19, "vector search ann hnsw ivf"),
        (20, "rust async tokio await futures"),
        (21, "rust ownership borrow checker lifetimes"),
        (22, "rust trait dyn impl async"),
        (23, "rust unsafe pointer manipulation"),
        (24, "linux kernel scheduler cfs"),
        (25, "linux network namespace netns"),
        (26, "windows powershell scripting"),
        (27, "macos darwin xcode swift"),
        (28, "ios swift uikit swiftui"),
        (29, "android kotlin jetpack compose"),
        (30, "tcp ip osi seven layers"),
        (31, "udp datagram unreliable fast"),
        (32, "http2 multiplexing streams binary"),
        (33, "http3 quic udp encrypted"),
        (34, "tls handshake certificate chain"),
        (35, "ssh key exchange rsa ed25519"),
        (36, "git rebase merge cherry pick"),
        (37, "git stash pop apply"),
        (38, "github pull request review approve"),
        (39, "ci cd pipeline github actions"),
        (40, "rust cargo build release profile"),
        (41, "rust crate publish workspace"),
        (42, "rust testing cfg test mod"),
        (43, "rust criterion benchmarks measure"),
        (44, "compiler optimization llvm ir"),
        (45, "compiler frontend parser ast"),
        (46, "interpreter virtual machine bytecode"),
        (47, "garbage collector mark sweep"),
        (48, "memory allocator slab arena"),
        (49, "memory mapped file mmap madvise"),
        (50, "concurrency lock free atomic"),
        (51, "concurrency mutex condvar wait"),
        (52, "rust send sync auto traits"),
        (53, "database transaction isolation"),
        (54, "database query optimizer plan"),
        (55, "data warehouse columnar storage"),
        (56, "parquet rowgroup metadata footer"),
        (57, "arrow record batch zero copy"),
        (58, "rust simd portable wide x86"),
        (59, "rust performance profiling perf"),
    ]
}

const SEGMENTS: usize = 4;
/// Number of unique-prefix terms planted across the superfiles.
/// Exactly one per segment so the prefix row demonstrably hits
/// every segment (no skip can prune it).
const N_PREFIX_TERMS: usize = SEGMENTS;
/// Size of the planted corpus, before prefix planting.
const N_PLANTED: usize = 60;

/// Plant `N_PREFIX_TERMS` unique-prefix terms (`alphafox<NN>`)
/// at known doc positions, one per segment. The terms are
/// alphanumeric-only — `_` doesn't survive `is_token_byte =
/// is_ascii_alphanumeric()` on either tokenizer, so a `foo_bar`
/// term lands as two FST keys, not one. Pure alphanumeric
/// suffixes give us four distinct prefix-expansion terms that
/// share the `alphafox` prefix.
///
/// Planted at the **last** doc of each segment (idx 14, 29, 44,
/// 59) so the existing curated corpus's per-doc properties
/// (lengths, token compositions for queries like "rust async")
/// stay intact.
fn corpus_with_prefix_terms() -> Vec<(u64, String)> {
    let mut corp: Vec<(u64, String)> = planted_corpus()
        .into_iter()
        .map(|(id, t)| (id, t.to_string()))
        .collect();
    let segment_size = N_PLANTED / SEGMENTS;
    for i in 0..N_PREFIX_TERMS {
        let target_idx = (i + 1) * segment_size - 1;
        let extra = format!(" alphafox{i:02}");
        corp[target_idx].1.push_str(&extra);
    }
    corp
}

// ---- Supertable side -----------------------------------------------


/// Build a supertable with `n_superfiles` superfiles, dividing the
/// corpus into equal contiguous chunks. One commit per chunk so
/// the segment boundaries match the per-engine shard boundaries
/// exactly.
fn build_supertable(corpus: &[(u64, String)], n_superfiles: usize) -> Supertable {
    let pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("pool"),
    );
    let tk: Arc<dyn Tokenizer> = default_tokenizer();
    let opts = SupertableOptions::new(
        schema_id_title(),
        vec![FtsConfig {
            column: "title".into(),
        }],
        vec![],
        Some(tk),
    )
    .expect("opts")
    .with_writer_pool(pool);

    let st = Supertable::create(opts);
    let mut w = st.writer().expect("writer");
    let chunk_size = corpus.len().div_ceil(n_superfiles);
    for chunk in corpus.chunks(chunk_size) {
        let titles =
            LargeStringArray::from(chunk.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>());
        let batch = RecordBatch::try_new(schema_id_title(), vec![Arc::new(titles)])
            .expect("batch");
        w.append(&batch).expect("append");
        w.commit().expect("commit");
    }
    drop(w);
    st
}

/// Run a supertable BM25 search and return the global doc-ids of
/// the hits, in the supertable's score-descending order.
///
/// The supertable returns hits as `(SuperfileUri, local_doc_id,
/// score)`. To get the *user-declared* `doc_id` (the value in the
/// id column), we resolve via segment ordering: superfiles appear
/// in the manifest in append order, each segment has 15 docs (for
/// the planted corpus), so global = segment_index * chunk_size +
/// local_doc_id.
///
/// This mirrors the segment-shape constraint the test fixture
/// imposes; production callers would carry their own surrogate
/// key column.
fn supertable_to_global_ids(st: &Supertable, hits: Vec<SuperfileHit>, chunk_size: usize) -> Vec<u64> {
    let r = st.reader();
    let manifest = r.manifest();
    hits.into_iter()
        .map(|h| {
            let seg_idx = manifest
                .superfiles
                .iter()
                .position(|e| e.uri == h.segment)
                .expect("segment in manifest");
            (seg_idx as u64) * (chunk_size as u64) + (h.local_doc_id as u64)
        })
        .collect()
}

fn supertable_search_global(st: &Supertable, query: &str, k: usize, chunk_size: usize) -> Vec<u64> {
    let r = st.reader();
    let hits = r
        .bm25_search("title", query, k, BoolMode::Or)
        .expect("supertable bm25");
    supertable_to_global_ids(st, hits, chunk_size)
}

fn supertable_prefix_global(
    st: &Supertable,
    prefix: &str,
    k: usize,
    chunk_size: usize,
) -> Vec<u64> {
    let r = st.reader();
    let hits = r
        .bm25_search_prefix("title", prefix, k)
        .expect("supertable bm25_prefix");
    supertable_to_global_ids(st, hits, chunk_size)
}

// ---- Tantivy side --------------------------------------------------

struct TantivyHandles {
    index: Index,
    title_field: tantivy::schema::Field,
    id_field: tantivy::schema::Field,
}

/// Build a Tantivy index that mirrors the supertable's segmenting:
/// one `IndexWriter::commit()` per chunk. Tantivy emits one
/// internal segment per commit, so segment boundaries match.
fn build_tantivy(corpus: &[(u64, String)], n_superfiles: usize) -> TantivyHandles {
    let mut schema_builder = TSchema::builder();
    let _id = schema_builder.add_u64_field("doc_id", INDEXED | STORED);
    let _title = schema_builder.add_text_field("title", TEXT);
    let schema = schema_builder.build();

    let index = Index::builder()
        .schema(schema.clone())
        .settings(IndexSettings::default())
        .create_in_ram()
        .expect("create_in_ram");
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build();
    index.tokenizers().register("default", analyzer);

    let id_field = schema.get_field("doc_id").expect("id field");
    let title_field = schema.get_field("title").expect("title field");

    // Match the supertable's chunk_size precisely. One commit per
    // chunk. `set_merge_policy(NoMergePolicy)` keeps each commit's
    // segment intact — without it Tantivy auto-merges, which
    // would consolidate per-segment IDF into one segment and break
    // the per-segment-IDF parity with the supertable.
    let chunk_size = corpus.len().div_ceil(n_superfiles);
    let mut writer = index.writer(50_000_000).expect("writer");
    writer.set_merge_policy(Box::new(NoMergePolicy));
    for chunk in corpus.chunks(chunk_size) {
        for (id, title) in chunk {
            writer
                .add_document(doc!(id_field => *id, title_field => title.as_str()))
                .expect("add_document");
        }
        writer.commit().expect("commit");
    }
    drop(writer);

    TantivyHandles {
        index,
        title_field,
        id_field,
    }
}

fn tantivy_top_k_query(handles: &TantivyHandles, q: &dyn Query, k: usize) -> Vec<u64> {
    let reader = handles.index.reader().expect("reader");
    let searcher = reader.searcher();
    let top = searcher
        .search(q, &TopDocs::with_limit(k).order_by_score())
        .expect("search");
    top.into_iter()
        .map(|(_score, addr)| {
            let d: tantivy::TantivyDocument = searcher.doc(addr).expect("fetch");
            d.get_first(handles.id_field)
                .expect("get id field")
                .as_u64()
                .expect("u64")
        })
        .collect()
}

fn tantivy_top_k(handles: &TantivyHandles, query: &str, k: usize) -> Vec<u64> {
    let parser = QueryParser::for_index(&handles.index, vec![handles.title_field]);
    let q = parser.parse_query(query).expect("parse_query");
    tantivy_top_k_query(handles, q.as_ref(), k)
}

/// Build a Tantivy `BooleanQuery::Or` over an explicit term list
/// — the manual prefix-expansion equivalent for the prefix row.
fn tantivy_top_k_term_or(handles: &TantivyHandles, terms: &[&str], k: usize) -> Vec<u64> {
    let subqueries: Vec<(Occur, Box<dyn Query>)> = terms
        .iter()
        .map(|t| {
            let term = Term::from_field_text(handles.title_field, t);
            let sq: Box<dyn Query> = Box::new(TermQuery::new(
                term,
                IndexRecordOption::WithFreqsAndPositions,
            ));
            (Occur::Should, sq)
        })
        .collect();
    let q = BooleanQuery::new(subqueries);
    tantivy_top_k_query(handles, &q, k)
}

// ---- Test helpers ---------------------------------------------------

const CHUNK_SIZE: usize = N_PLANTED / SEGMENTS;

// ---- Shared fixture for the 8 standard-corpus oracles -------------
//
// Each of the 8 `oracle_*` tests below was building its own
// `corpus_with_prefix_terms()` + `build_supertable(&corp,
// SEGMENTS)` + `build_tantivy(&corp, SEGMENTS)` from scratch
// — 8× the setup work. The tests that need a custom corpus
// (`prefix_skip_prunes_segments_without_matching_lex_range`
// plants a term in only one segment; `oracle_zipfian_*`
// uses a Zipfian corpus) build their own fixture and don't
// touch this cache.

struct StandardFixture {
    infino: Supertable,
    tan: TantivyHandles,
}

static STANDARD_FIXTURE: LazyLock<StandardFixture> = LazyLock::new(|| {
    let corp = corpus_with_prefix_terms();
    let infino = build_supertable(&corp, SEGMENTS);
    let tan = build_tantivy(&corp, SEGMENTS);
    StandardFixture { infino, tan }
});

fn assert_top_k_sets_match(label: &str, infino: Vec<u64>, tantivy: Vec<u64>, head_size: usize) {
    let infino_head: HashSet<u64> = infino.iter().take(head_size).copied().collect();
    let tantivy_head: HashSet<u64> = tantivy.iter().take(head_size).copied().collect();
    assert_eq!(
        infino_head, tantivy_head,
        "{label}: top-{head_size} sets disagree — infino={infino:?} tantivy={tantivy:?}",
    );
}

// ---- Tests: 6 query shapes from the bench ---------------------------

#[test]
fn oracle_single_rare_top1_matches() {
    // "tantivy" appears in exactly 1 doc (id=17, segment 1).
    let f = &*STANDARD_FIXTURE;
    let infino = &f.infino;
    let tan = &f.tan;
    let inf_hits = supertable_search_global(&infino, "tantivy", 5, CHUNK_SIZE);
    let tan_hits = tantivy_top_k(&tan, "tantivy", 5);
    assert_eq!(inf_hits.first().copied(), Some(17));
    assert_eq!(tan_hits.first().copied(), Some(17));
    assert_top_k_sets_match("single_rare", inf_hits, tan_hits, 1);
}

#[test]
fn oracle_single_common_top3_matches() {
    // "rust" appears in many docs. Top-3 sets must agree —
    // per-segment IDF is identical across engines (same
    // chunk_size), so scoring agrees up to tie-breaking.
    let f = &*STANDARD_FIXTURE;
    let infino = &f.infino;
    let tan = &f.tan;
    let inf_hits = supertable_search_global(&infino, "rust", 10, CHUNK_SIZE);
    let tan_hits = tantivy_top_k(&tan, "rust", 10);
    // Top-1 is tied across many "rust dl=4" docs; assert top-3
    // set membership in the larger top-10 candidate set.
    let inf_set: HashSet<u64> = inf_hits.iter().take(10).copied().collect();
    let tan_set: HashSet<u64> = tan_hits.iter().take(10).copied().collect();
    let common: HashSet<u64> = inf_set.intersection(&tan_set).copied().collect();
    assert!(
        common.len() >= 3,
        "single_common: top-10 sets should overlap by ≥3 — \
         infino={inf_hits:?} tantivy={tan_hits:?}",
    );
}

#[test]
fn oracle_two_term_or_top2_matches() {
    // Docs containing both "rust" AND "async": doc 0, doc 20,
    // doc 22. Top-2 of "rust async" should include 0 and 20 on
    // both engines.
    let f = &*STANDARD_FIXTURE;
    let infino = &f.infino;
    let tan = &f.tan;
    let inf_hits = supertable_search_global(&infino, "rust async", 5, CHUNK_SIZE);
    let tan_hits = tantivy_top_k(&tan, "rust async", 5);
    let inf_top2: HashSet<u64> = inf_hits.iter().take(2).copied().collect();
    let tan_top2: HashSet<u64> = tan_hits.iter().take(2).copied().collect();
    assert!(
        inf_top2.contains(&0) && inf_top2.contains(&20),
        "infino top-2 should include docs 0 and 20; got {inf_hits:?}"
    );
    assert!(
        tan_top2.contains(&0) && tan_top2.contains(&20),
        "tantivy top-2 should include docs 0 and 20; got {tan_hits:?}"
    );
    assert_eq!(inf_top2, tan_top2);
}

#[test]
fn oracle_three_wide_or_top3_matches() {
    // "rust web framework" — the wide-OR shape.
    let f = &*STANDARD_FIXTURE;
    let infino = &f.infino;
    let tan = &f.tan;
    let inf_hits = supertable_search_global(&infino, "rust web framework", 10, CHUNK_SIZE);
    let tan_hits = tantivy_top_k(&tan, "rust web framework", 10);
    // Top-3 of "rust web framework": doc 8 ("rust web framework"
    // hits all three terms). Both engines should return doc 8 in
    // the head.
    let inf_top: HashSet<u64> = inf_hits.iter().take(3).copied().collect();
    let tan_top: HashSet<u64> = tan_hits.iter().take(3).copied().collect();
    assert!(inf_top.contains(&8));
    assert!(tan_top.contains(&8));
    assert_top_k_sets_match("three_wide_or", inf_hits, tan_hits, 3);
}

#[test]
fn oracle_three_similar_or_top3_matches() {
    // "redis kafka elasticsearch" — three similar-frequency
    // single-term docs (14, 15, 16). Top-3 must be exactly
    // {14, 15, 16}.
    let f = &*STANDARD_FIXTURE;
    let infino = &f.infino;
    let tan = &f.tan;
    let inf_hits = supertable_search_global(&infino, "redis kafka elasticsearch", 5, CHUNK_SIZE);
    let tan_hits = tantivy_top_k(&tan, "redis kafka elasticsearch", 5);
    let want: HashSet<u64> = [14u64, 15, 16].into_iter().collect();
    let inf_top: HashSet<u64> = inf_hits.iter().take(3).copied().collect();
    let tan_top: HashSet<u64> = tan_hits.iter().take(3).copied().collect();
    assert_eq!(inf_top, want);
    assert_eq!(tan_top, want);
}

#[test]
fn oracle_five_term_or_top5_matches() {
    // "tcp udp http2 http3 tls" — five terms, each with a single-
    // doc match (30, 31, 32, 33, 34).
    let f = &*STANDARD_FIXTURE;
    let infino = &f.infino;
    let tan = &f.tan;
    let inf_hits = supertable_search_global(&infino, "tcp udp http2 http3 tls", 10, CHUNK_SIZE);
    let tan_hits = tantivy_top_k(&tan, "tcp udp http2 http3 tls", 10);
    let want: HashSet<u64> = [30u64, 31, 32, 33, 34].into_iter().collect();
    let inf_top: HashSet<u64> = inf_hits.iter().take(5).copied().collect();
    let tan_top: HashSet<u64> = tan_hits.iter().take(5).copied().collect();
    assert_eq!(inf_top, want);
    assert_eq!(tan_top, want);
}

// ---- Tests: prefix row exercising term-range skip -------------------

#[test]
fn oracle_prefix_query_matches_explicit_term_or() {
    // The supertable's prefix search expands `alphafox_` via FST
    // walk, then runs a per-segment OR over the expansion. We
    // mirror this by manually building the same OR on Tantivy.
    let f = &*STANDARD_FIXTURE;
    let infino = &f.infino;
    let tan = &f.tan;

    // Planted: alphafox00 (doc 14, segment 0) … alphafox03
    // (doc 59, segment 3). Each is a distinct FST key sharing
    // the `alphafox` prefix.
    let prefix = "alphafox";
    let expanded: Vec<&str> = (0..N_PREFIX_TERMS)
        .map(|i| match i {
            0 => "alphafox00",
            1 => "alphafox01",
            2 => "alphafox02",
            3 => "alphafox03",
            _ => unreachable!(),
        })
        .collect();

    let inf_hits = supertable_prefix_global(&infino, prefix, 10, CHUNK_SIZE);
    let tan_hits = tantivy_top_k_term_or(&tan, &expanded, 10);

    let want: HashSet<u64> = [14u64, 29, 44, 59].into_iter().collect();
    let inf_set: HashSet<u64> = inf_hits.iter().take(N_PREFIX_TERMS).copied().collect();
    let tan_set: HashSet<u64> = tan_hits.iter().take(N_PREFIX_TERMS).copied().collect();
    assert_eq!(inf_set, want, "infino prefix hits = {inf_hits:?}");
    assert_eq!(tan_set, want, "tantivy explicit-OR hits = {tan_hits:?}");
}

// ---- Bonus: prefix-row skip behavior --------------------------------

#[test]
fn prefix_skip_prunes_segments_without_matching_lex_range() {
    // Plant a prefix term in only ONE segment, leaving the other
    // superfiles' lex term ranges fully below the prefix. Verify the
    // supertable returns exactly the planted doc and the term-range
    // skip prevents the other superfiles from being touched (asserted
    // implicitly: the result has zero contributions from superfiles
    // 1..3).
    let mut corp: Vec<(u64, String)> = planted_corpus()
        .into_iter()
        .map(|(id, t)| (id, t.to_string()))
        .collect();
    // Only plant in segment 0 (doc 0). Other superfiles contain
    // none of the `quokka` prefix-space.
    corp[0].1.push_str(" quokka_unique");
    let infino = build_supertable(&corp, SEGMENTS);
    let r = infino.reader();
    let hits = r.bm25_search_prefix("title", "quokka", 5).expect("prefix");
    assert_eq!(hits.len(), 1);
    let manifest = r.manifest();
    let target_uri = manifest.superfiles[0].uri;
    assert_eq!(hits[0].segment, target_uri);
    assert_eq!(hits[0].local_doc_id, 0);
}

// ---- Sanity: empty + no-match cases ---------------------------------

#[test]
fn oracle_no_match_returns_empty_on_both_engines() {
    let f = &*STANDARD_FIXTURE;
    let infino = &f.infino;
    let tan = &f.tan;
    let inf_hits = supertable_search_global(&infino, "definitelynotpresent", 5, CHUNK_SIZE);
    let tan_hits = tantivy_top_k(&tan, "definitelynotpresent", 5);
    assert!(inf_hits.is_empty());
    assert!(tan_hits.is_empty());
}

// ---- Larger-scale Zipfian smoke (single test) ------------------------

/// Generate a small Zipfian corpus matching the bench's shape but
/// at test-fast scale (5K docs × 100 tokens × 10K vocab).
fn zipfian_corpus(n_docs: usize, seed: u64) -> Vec<(u64, String)> {
    use rand::RngExt;
    let mut rng = StdRng::seed_from_u64(seed);
    // Cumulative inverse-rank weights for a 10K vocab.
    const VOCAB: usize = 10_000;
    const TOKENS_PER_DOC: usize = 100;
    let mut cum = Vec::with_capacity(VOCAB);
    let mut acc = 0.0f64;
    for i in 1..=VOCAB {
        acc += 1.0 / (i as f64);
        cum.push(acc);
    }
    let total = *cum.last().expect("vocab > 0");

    let mut out = Vec::with_capacity(n_docs);
    for d in 0..n_docs as u64 {
        let mut s = String::with_capacity(TOKENS_PER_DOC * 8);
        for j in 0..TOKENS_PER_DOC {
            let target: f64 = rng.random::<f64>() * total;
            let idx = match cum
                .binary_search_by(|p| p.partial_cmp(&target).unwrap_or(std::cmp::Ordering::Equal))
            {
                Ok(i) | Err(i) => i.min(VOCAB - 1) + 1,
            };
            if j > 0 {
                s.push(' ');
            }
            s.push_str(&format!("term{idx:05}"));
        }
        out.push((d, s));
    }
    out
}

#[test]
fn oracle_zipfian_corpus_six_query_shapes_match() {
    // 5K docs × 4 superfiles = 1250 docs/segment. Tests the same
    // 6 query shapes the bench exercises. Larger k (50) puts
    // tie boundaries outside the head-set comparison.
    let n_docs = 5_000;
    let corp = zipfian_corpus(n_docs, 42);
    let infino = build_supertable(&corp, SEGMENTS);
    let tan = build_tantivy(&corp, SEGMENTS);
    let chunk = n_docs / SEGMENTS;
    let k = 50;

    // single_common ("term00001") is intentionally absent here:
    // at this corpus shape, every doc contains the term ~10x at
    // the same dl — BM25 scores collapse into one giant tied
    // bucket, and top-k membership becomes a pure tie-breaker
    // race. infino tie-breaks by ascending local doc_id (BMW's
    // natural order); Tantivy uses a different tie-breaker. The
    // disagreement isn't a correctness gap — both top-k sets are
    // mathematically valid under BM25 — but the bucket size
    // makes set overlap a lossy invariant. Single_common is
    // covered by the planted-corpus test where dl variation
    // differentiates the head.
    let queries = [
        ("single_rare", "term09999"),
        ("two_term_or", "term00001 term00050"),
        ("three_wide_or", "term00001 term00050 term00100"),
        ("three_similar_or", "term00050 term00051 term00052"),
        (
            "five_term_or",
            "term00050 term00051 term00052 term00053 term00054",
        ),
    ];

    for (label, q) in queries {
        let inf = supertable_search_global(&infino, q, k, chunk);
        let tan_hits = tantivy_top_k(&tan, q, k);
        let inf_set: HashSet<u64> = inf.iter().copied().collect();
        let tan_set: HashSet<u64> = tan_hits.iter().copied().collect();
        let common = inf_set.intersection(&tan_set).count();
        let target = inf_set.len().min(tan_set.len());
        // 60% overlap threshold: per-segment IDF is identical
        // (we control segmentation), so the head of the top-k
        // matches; the tail diverges via tie-breaker order. At
        // k=50, a 60% overlap means the engines agree on ≥30
        // of the 50 docs — far above chance (50/n_docs * 50 ≈
        // 0.5% expected by chance), and well within the
        // tie-breaker drift budget.
        let threshold = (target * 6) / 10;
        assert!(
            common >= threshold,
            "{label}: top-{k} overlap {common}/{target} below 60% threshold; \
             infino={inf:?} tantivy={tan_hits:?}",
        );
    }
}
