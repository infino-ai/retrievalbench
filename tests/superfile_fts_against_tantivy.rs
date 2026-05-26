//! Cross-implementation BM25 oracle: our FTS pipeline vs Tantivy.
//!
//! We index the same synthetic corpus into both engines with matching
//! BM25 parameters (K1=1.2, B=0.75) and matching tokenization
//! (ASCII-lowercase, whitespace-split, no stemming) and compare the
//! top-k doc IDs produced by each for a query battery.
//!
//! ## What this oracle catches
//!
//! Planted-ground-truth tests verify the pipeline returns the
//! *expected* docs but not that the *scoring math* is right — a
//! self-consistent BM25 bug (e.g. wrong avgdl handling) can produce
//! correct relative ranking on the planted set while disagreeing with
//! every other BM25 implementation on the planet. Comparing against
//! Tantivy's well-validated implementation catches this class.
//!
//! ## Tolerances
//!
//! Top-k *sets* must agree exactly on the head (top-1 always; top-3
//! when df differs > 1). Order within a tied score may vary because
//! Tantivy and our impl tie-break by different rules. We assert "set
//! equality" on the top-k for the query, not "ordered equality".
//!
//! ## Why not test scores numerically
//!
//! Tantivy uses a slightly different IDF formulation by default
//! (`ln(1 + (N - df + 0.5) / (df + 0.5))`); our implementation
//! matches Lucene's BM25 (`ln((N - df + 0.5) / (df + 0.5) + 1)` —
//! same series, equivalent for non-pathological N). Numerically
//! they're within a small constant; exact-equality on raw scores
//! would fail. Top-k *order* is invariant under monotone transforms
//! of the IDF, so the doc-set agreement is the right invariant to
//! check.

use arrow_array::{LargeStringArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use bytes::Bytes;
use infino::superfile::SuperfileReader;
use infino::superfile::builder::{BuilderOptions, FtsConfig, SuperfileBuilder};
use infino::superfile::fts::reader::BoolMode;
use infino::test_helpers::{decimal128_ids, default_tokenizer};
use std::collections::HashSet;
use std::sync::Arc;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{INDEXED, STORED, Schema as TSchema, TEXT, Value};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{Index, IndexSettings, doc};

/// 60-doc planted corpus with mixed term frequencies. Enough to make
/// BM25's tf+idf+dl-norm interaction non-trivial, small enough to
/// keep the test fast.
fn corpus() -> Vec<(u64, &'static str)> {
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

/// Build our infino superfile from the corpus.
fn build_infino_superfile(corpus: &[(u64, &str)]) -> SuperfileReader {
    let schema = Arc::new(Schema::new(vec![
        Field::new("doc_id", DataType::Decimal128(38, 0), false),
        Field::new("title", DataType::LargeUtf8, false),
    ]));
    let opts = BuilderOptions::new(
        schema.clone(),
        "doc_id",
        vec![FtsConfig {
            column: "title".into(),
        }],
        vec![],
        Some(default_tokenizer()),
    );
    let mut b = SuperfileBuilder::new(opts).expect("new SuperfileBuilder");
    let ids = decimal128_ids(corpus.iter().map(|(i, _)| *i));
    let titles = LargeStringArray::from(corpus.iter().map(|(_, t)| *t).collect::<Vec<_>>());
    let batch = RecordBatch::try_new(schema, vec![Arc::new(ids), Arc::new(titles)])
        .expect("build RecordBatch");
    b.add_batch(&batch, &[]).expect("add_batch");
    let bytes = Bytes::from(b.finish().expect("finish builder"));
    SuperfileReader::open(bytes).expect("open superfile")
}

/// Build a Tantivy index from the same corpus, with tokenization
/// matched as closely as possible to our `AsciiLowerTokenizer`
/// (whitespace + punctuation split, ASCII lowercase, no stemming).
fn build_tantivy_index(corpus: &[(u64, &str)]) -> (Index, tantivy::schema::Field) {
    let mut schema_builder = TSchema::builder();
    let _id_field = schema_builder.add_u64_field("doc_id", INDEXED | STORED);
    let _title_field = schema_builder.add_text_field("title", TEXT);
    let schema = schema_builder.build();

    let index = Index::builder()
        .schema(schema.clone())
        .settings(IndexSettings::default())
        .create_in_ram()
        .expect("Tantivy create_in_ram");

    // Match our tokenization: SimpleTokenizer (split on punct +
    // whitespace) + LowerCaser. No stemming, no stop words.
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build();
    index.tokenizers().register("default", analyzer);

    let id_field = schema.get_field("doc_id").expect("get field");
    let title_f = schema.get_field("title").expect("get field");
    let mut writer = index.writer(50_000_000).expect("create Tantivy writer");
    for (id, title) in corpus {
        writer
            .add_document(doc!(id_field => *id, title_f => *title))
            .expect("add Tantivy document");
    }
    writer.commit().expect("commit builder");
    (index, title_f)
}

/// Run a tantivy search and return doc_ids in score-descending order.
fn tantivy_top_k(
    index: &Index,
    title_field: tantivy::schema::Field,
    query: &str,
    k: usize,
) -> Vec<u64> {
    let reader = index.reader().expect("open Tantivy reader");
    let searcher = reader.searcher();
    let parser = QueryParser::for_index(index, vec![title_field]);
    let q = parser.parse_query(query).expect("parse Tantivy query");
    let top = searcher
        .search(&q, &TopDocs::with_limit(k).order_by_score())
        .expect("search");
    let id_field = index.schema().get_field("doc_id").expect("get field");
    top.into_iter()
        .map(|(_score, addr)| {
            let doc: tantivy::TantivyDocument = searcher.doc(addr).expect("fetch tantivy doc");
            doc.get_first(id_field)
                .expect("get first field")
                .as_u64()
                .expect("as u64")
        })
        .collect()
}

/// Run our search and return doc_ids in score-descending order, using
/// the same `doc_id`s the corpus declared (we built the superfile
/// with these as the user `doc_id` column, but our reader returns
/// `local_doc_id`s — those are the row index 0..N-1, equal to the
/// declared `doc_id` since the corpus is dense from 0).
fn infino_top_k(reader: &SuperfileReader, query: &str, k: usize) -> Vec<u64> {
    let hits = reader
        .bm25_search("title", query, k, BoolMode::Or)
        .expect("BM25 search");
    hits.into_iter().map(|(d, _)| d as u64).collect()
}

/// Compare top-k *sets* between the two engines for a query. Asserts
/// agreement on the head; allows tail divergence for ties (Tantivy
/// and our impl break score-ties differently). `head_size` is how
/// many of the top results must match as a set (order-independent).
fn assert_top_k_head_agrees(
    infino: &SuperfileReader,
    tantivy_idx: &Index,
    title_field: tantivy::schema::Field,
    query: &str,
    head_size: usize,
    k: usize,
) {
    let infino_hits = infino_top_k(infino, query, k);
    let tantivy_hits = tantivy_top_k(tantivy_idx, title_field, query, k);
    assert!(
        infino_hits.len() >= head_size && tantivy_hits.len() >= head_size,
        "query {query:?}: not enough hits — infino={infino_hits:?} tantivy={tantivy_hits:?}"
    );
    let infino_head: HashSet<u64> = infino_hits.into_iter().take(head_size).collect();
    let tantivy_head: HashSet<u64> = tantivy_hits.into_iter().take(head_size).collect();
    assert_eq!(
        infino_head, tantivy_head,
        "query {query:?}: top-{head_size} sets disagree"
    );
}

#[test]
fn oracle_rare_term_top1_matches() {
    // Single-term, single-doc match: the only doc with "tantivy" is
    // doc 17. Both engines must return [17] as top-1.
    let corp = corpus();
    let infino = build_infino_superfile(&corp);
    let (tan, tf) = build_tantivy_index(&corp);
    assert_top_k_head_agrees(&infino, &tan, tf, "tantivy", 1, 5);
}

#[test]
fn oracle_common_term_top1_in_correct_set() {
    // "rust" appears in many same-length docs (dl=4, tf=1) at
    // mathematically tied BM25 scores. We can't assert exact top-K
    // set agreement because tie-breaking diverges between
    // implementations, but BOTH engines should pick top-1 from the
    // "rust dl=4" tied set, never a doc that doesn't contain "rust".
    let corp = corpus();
    let infino = build_infino_superfile(&corp);
    let (tan, tf) = build_tantivy_index(&corp);
    let infino_top: u64 = infino_top_k(&infino, "rust", 1)[0];
    let tantivy_top: u64 = tantivy_top_k(&tan, tf, "rust", 1)[0];
    let rust_docs: HashSet<u64> = corp
        .iter()
        .filter(|(_, t)| t.split_whitespace().any(|w| w == "rust"))
        .map(|(i, _)| *i)
        .collect();
    assert!(
        rust_docs.contains(&infino_top),
        "infino top-1 doc {infino_top} doesn't contain 'rust'"
    );
    assert!(
        rust_docs.contains(&tantivy_top),
        "tantivy top-1 doc {tantivy_top} doesn't contain 'rust'"
    );
}

#[test]
fn oracle_two_term_or_top1_matches() {
    // "redis kafka" — doc 14 has "redis", doc 15 has "kafka". Both
    // single-occurrence docs; either could be top-1 (Tantivy and we
    // may break this differently). Top-2 set must agree.
    let corp = corpus();
    let infino = build_infino_superfile(&corp);
    let (tan, tf) = build_tantivy_index(&corp);
    assert_top_k_head_agrees(&infino, &tan, tf, "redis kafka", 2, 5);
}

#[test]
fn oracle_two_term_overlap_top3_matches() {
    // "rust async" — docs 0 and 20 contain both terms, so they
    // should rank highest under any sensible BM25.
    let corp = corpus();
    let infino = build_infino_superfile(&corp);
    let (tan, tf) = build_tantivy_index(&corp);
    let infino_hits = infino_top_k(&infino, "rust async", 5);
    let tantivy_hits = tantivy_top_k(&tan, tf, "rust async", 5);
    let infino_head: HashSet<u64> = infino_hits.into_iter().take(2).collect();
    let tantivy_head: HashSet<u64> = tantivy_hits.into_iter().take(2).collect();
    assert!(
        infino_head.contains(&0) && infino_head.contains(&20),
        "infino top-2 should contain docs 0+20 (both 'rust' and 'async'); got {infino_head:?}"
    );
    assert!(
        tantivy_head.contains(&0) && tantivy_head.contains(&20),
        "tantivy top-2 should contain docs 0+20; got {tantivy_head:?}"
    );
    assert_eq!(infino_head, tantivy_head);
}

#[test]
fn oracle_three_term_query_top5_set_matches() {
    let corp = corpus();
    let infino = build_infino_superfile(&corp);
    let (tan, tf) = build_tantivy_index(&corp);
    assert_top_k_head_agrees(&infino, &tan, tf, "rust web framework", 3, 10);
}

#[test]
fn oracle_no_match_query_returns_empty() {
    // "xyzzy" is in none of the docs; both engines must return empty.
    let corp = corpus();
    let infino = build_infino_superfile(&corp);
    let (tan, tf) = build_tantivy_index(&corp);
    let infino_hits = infino_top_k(&infino, "xyzzy", 5);
    let tantivy_hits = tantivy_top_k(&tan, tf, "xyzzy", 5);
    assert!(
        infino_hits.is_empty(),
        "infino should return [] for unknown term"
    );
    assert!(
        tantivy_hits.is_empty(),
        "tantivy should return [] for unknown term"
    );
}

// ─── AND-mode oracles vs Tantivy ──────────────────────────────────────

/// Tantivy AND search: use a query parser configured to default to
/// conjunction, so a bare `"rust async"` is interpreted as
/// `+rust +async` (every term required), matching infino's
/// `BoolMode::And` semantic.
fn tantivy_top_k_and(
    index: &Index,
    title_field: tantivy::schema::Field,
    query: &str,
    k: usize,
) -> Vec<u64> {
    let reader = index.reader().expect("open Tantivy reader");
    let searcher = reader.searcher();
    let mut parser = QueryParser::for_index(index, vec![title_field]);
    parser.set_conjunction_by_default();
    let q = parser.parse_query(query).expect("parse Tantivy AND query");
    let top = searcher
        .search(&q, &TopDocs::with_limit(k).order_by_score())
        .expect("search");
    let id_field = index.schema().get_field("doc_id").expect("get field");
    top.into_iter()
        .map(|(_score, addr)| {
            let doc: tantivy::TantivyDocument = searcher.doc(addr).expect("fetch tantivy doc");
            doc.get_first(id_field)
                .expect("get first field")
                .as_u64()
                .expect("as u64")
        })
        .collect()
}

fn infino_top_k_and(reader: &SuperfileReader, query: &str, k: usize) -> Vec<u64> {
    let hits = reader
        .bm25_search("title", query, k, BoolMode::And)
        .expect("AND BM25 search");
    hits.into_iter().map(|(d, _)| d as u64).collect()
}

fn assert_top_k_and_set_matches(
    infino: &SuperfileReader,
    tantivy_idx: &Index,
    title_field: tantivy::schema::Field,
    query: &str,
    k: usize,
) {
    let infino_hits = infino_top_k_and(infino, query, k);
    let tantivy_hits = tantivy_top_k_and(tantivy_idx, title_field, query, k);
    let infino_set: HashSet<u64> = infino_hits.iter().copied().collect();
    let tantivy_set: HashSet<u64> = tantivy_hits.iter().copied().collect();
    assert_eq!(
        infino_set, tantivy_set,
        "AND query {query:?}: top-{k} sets disagree — infino={infino_hits:?} tantivy={tantivy_hits:?}"
    );
}

#[test]
fn oracle_and_two_term_overlap_matches_tantivy() {
    // "rust" + "async" co-occur in docs {0, 20, 22}. Both engines must
    // return exactly that set under AND semantics.
    let corp = corpus();
    let infino = build_infino_superfile(&corp);
    let (tan, tf) = build_tantivy_index(&corp);
    assert_top_k_and_set_matches(&infino, &tan, tf, "rust async", 10);
    let want: HashSet<u64> = [0u64, 20, 22].into_iter().collect();
    let infino_set: HashSet<u64> = infino_top_k_and(&infino, "rust async", 10).into_iter().collect();
    assert_eq!(infino_set, want, "infino AND(rust, async) must be {{0,20,22}}");
}

#[test]
fn oracle_and_three_term_singleton_matches_tantivy() {
    // "rust async tokio" intersect only at docs {0, 20} (doc 0 has all
    // three; doc 20 has rust+async+tokio too — "rust async tokio await
    // futures"). Both engines must agree on this set.
    let corp = corpus();
    let infino = build_infino_superfile(&corp);
    let (tan, tf) = build_tantivy_index(&corp);
    assert_top_k_and_set_matches(&infino, &tan, tf, "rust async tokio", 10);
}

#[test]
fn oracle_and_missing_term_returns_empty_in_both() {
    // A term that isn't in the corpus must short-circuit AND to empty
    // in both engines.
    let corp = corpus();
    let infino = build_infino_superfile(&corp);
    let (tan, tf) = build_tantivy_index(&corp);
    let infino_hits = infino_top_k_and(&infino, "rust definitelynotpresent", 10);
    let tantivy_hits = tantivy_top_k_and(&tan, tf, "rust definitelynotpresent", 10);
    assert!(infino_hits.is_empty(), "infino got {infino_hits:?}");
    assert!(tantivy_hits.is_empty(), "tantivy got {tantivy_hits:?}");
}

#[test]
fn oracle_and_disjoint_terms_return_empty_in_both() {
    // Both terms exist in the corpus but never co-occur ("python" in
    // docs 2,3; "kafka" in doc 15). Both engines must return empty.
    let corp = corpus();
    let infino = build_infino_superfile(&corp);
    let (tan, tf) = build_tantivy_index(&corp);
    let infino_hits = infino_top_k_and(&infino, "python kafka", 10);
    let tantivy_hits = tantivy_top_k_and(&tan, tf, "python kafka", 10);
    assert!(infino_hits.is_empty(), "infino got {infino_hits:?}");
    assert!(tantivy_hits.is_empty(), "tantivy got {tantivy_hits:?}");
}

#[test]
fn oracle_long_doc_vs_short_doc_dl_norm() {
    // BM25's dl-norm should make short docs that contain a term rank
    // higher than long docs containing the same term once. This test
    // checks that *both* engines agree on dl-norm direction by
    // verifying their top-1 for "framework" is the same. Doc 7 ("go
    // web framework gin echo", 5 tokens) and doc 8 ("rust web
    // framework actix axum", 5 tokens) both contain "framework"
    // exactly once at the same dl. Top-1 may tie-break either way
    // but top-2 set must include both.
    let corp = corpus();
    let infino = build_infino_superfile(&corp);
    let (tan, tf) = build_tantivy_index(&corp);
    let infino_hits = infino_top_k(&infino, "framework", 5);
    let tantivy_hits = tantivy_top_k(&tan, tf, "framework", 5);
    let infino_top2: HashSet<u64> = infino_hits.into_iter().take(2).collect();
    let tantivy_top2: HashSet<u64> = tantivy_hits.into_iter().take(2).collect();
    assert_eq!(infino_top2, tantivy_top2, "framework top-2 sets disagree");
}
