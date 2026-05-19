//! Single-binary FTS bench for the superfile layer:
//!
//!   ingest head-to-head infino vs Tantivy
//! + 7-query search head-to-head infino vs Tantivy
//! + 3 per-algorithm (WAND+BMW / MaxScore+BMM) probes on infino
//! + correctness gates (BMW-vs-brute-force on infino, df=1 cross-engine
//!   against Tantivy)
//!
//! Pinned to 1M-doc Zipfian (200 tokens/doc, 10K vocab). The
//! single-superfile shape is rarely much larger in production —
//! supertable scale-out via N superfiles is what `benches/fts/supertable.rs`
//! stresses at 10M+ docs.
//!
//! Invocation:
//!
//! ```text
//! cargo bench --bench fts                          # both ingest + search across both topics
//! cargo bench --bench fts -- superfile_fts_build             # only superfile ingest timing
//! cargo bench --bench fts -- superfile_fts_search            # only superfile search timing
//! cargo bench --bench fts -- _build                # ingest across superfile + supertable
//! cargo bench --bench fts -- _search               # search across superfile + supertable
//! ```
//!
//! Correctness phase runs unconditionally on every invocation
//! (criterion filters skip timing, not setup), so a filter to
//! `superfile_fts_search` still validates the BMW oracle + df=1 cross-engine
//! match before timing kicks in.

use bytes::Bytes;
use criterion::{BenchmarkGroup, Criterion, Throughput, criterion_group, measurement::WallTime};
use infino::superfile::fts::builder::FtsBuilder;
use infino::superfile::fts::reader::{BoolMode, FtsReader, OrAlgo};
use infino::test_helpers::default_tokenizer;
use rayon::prelude::*;
use std::hint::black_box;
use std::sync::OnceLock;
use tantivy::Index;
use tantivy::collector::TopDocs;
use tantivy::doc;
use tantivy::query::{Query, QueryParser};
use tantivy::schema::{
    INDEXED, IndexRecordOption, STORED, Schema, TextFieldIndexing, TextOptions,
};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};

// ─── Constants ────────────────────────────────────────────────────────

/// Tantivy heap budget for the indexer. Our indexed payload at 1M
/// docs × 200 tokens × 9 bytes/token ≈ 1.8 GB raw text; Tantivy
/// internally compresses to ~150 MB. 500 MB is comfortable headroom
/// without spilling superfiles mid-bench.
const TANTIVY_HEAP_BYTES: usize = 500_000_000;

/// Single-column FTS schema JSON, shared between every infino blob
/// the FTS-superfile benches open.
const FTS_COLUMNS_JSON: &str = r#"[{"name":"title","tokenizer":"ascii_lower"}]"#;

/// Doc count for every FTS-superfile bench. Pinned to 1M — a single
/// superfile in production is rarely much larger; the multi-segment
/// supertable shape (which scales linearly via superfiles) is what
/// `INFINO_BENCH_FULL=1 cargo bench --bench vector` and the scale
/// bundle stress at 10M+ documents.
const N_DOCS: usize = 1_000_000;

enum TantivyThreads {
    Single,
    Default,
}

struct TantivyHandles {
    index: Index,
    title_field: tantivy::schema::Field,
}

// ─── Fixtures (built once, reused across criterion samples) ────────────

static DOCS: OnceLock<Vec<String>> = OnceLock::new();
static INFINO_BLOB: OnceLock<Vec<u8>> = OnceLock::new();
static TANTIVY: OnceLock<TantivyHandles> = OnceLock::new();

fn docs() -> &'static [String] {
    DOCS.get_or_init(|| crate::corpus::generate_text_corpus(N_DOCS, 1))
        .as_slice()
}

fn infino_reader() -> FtsReader {
    let blob = INFINO_BLOB.get_or_init(|| build_infino_blob_1thread(docs()));
    open_infino(blob)
}

fn tantivy_handles() -> &'static TantivyHandles {
    TANTIVY.get_or_init(|| build_tantivy(docs(), TantivyThreads::Single))
}

// ─── Builders — infino ────────────────────────────────────────────────

/// Build a single-threaded FTS blob from `docs`. Used both as the
/// correctness fixture and as the body of the `infino_1thread`
/// ingest-timing closure.
fn build_infino_blob_1thread(docs: &[String]) -> Vec<u8> {
    let mut builder = FtsBuilder::new(default_tokenizer());
    builder
        .register_column("title".to_string())
        .expect("register column");
    for (i, text) in docs.iter().enumerate() {
        builder.add_doc(0, i as u32, text).expect("add doc");
    }
    builder.finish()
}

/// Rayon-sharded parallel build. Each shard runs its own
/// `FtsBuilder` and emits a self-contained FTS blob — composes
/// with `SuperfileBuilder::commit()`'s multi-segment output shape.
fn build_infino_blobs_rayon(docs: &[String]) -> Vec<Vec<u8>> {
    let n_shards = rayon::current_num_threads();
    let docs_per_shard = docs.len().div_ceil(n_shards);
    docs.chunks(docs_per_shard)
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(build_infino_blob_1thread)
        .collect()
}

fn open_infino(blob: &[u8]) -> FtsReader {
    FtsReader::open(Bytes::from(blob.to_vec()), FTS_COLUMNS_JSON).expect("open FtsReader")
}

// ─── Builders — Tantivy ───────────────────────────────────────────────

/// Build a Tantivy index over `docs`. `WithFreqs` (no positions) —
/// matches infino's `(doc_id, tf)`-only posting layout.
fn build_tantivy(docs: &[String], threads: TantivyThreads) -> TantivyHandles {
    let mut schema_builder = Schema::builder();
    let id_field = schema_builder.add_u64_field("doc_id", INDEXED | STORED);
    let title_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(IndexRecordOption::WithFreqs),
    );
    let title_field = schema_builder.add_text_field("title", title_opts);
    let schema = schema_builder.build();
    let index = Index::builder()
        .schema(schema)
        .create_in_ram()
        .expect("create_in_ram");
    // SimpleTokenizer + LowerCaser ≈ infino's AsciiLowerTokenizer —
    // identical token streams on our alnum-only corpus.
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build();
    index.tokenizers().register("default", analyzer);

    let mut writer = match threads {
        TantivyThreads::Single => index
            .writer_with_num_threads(1, TANTIVY_HEAP_BYTES)
            .expect("writer 1-thread"),
        TantivyThreads::Default => index.writer(TANTIVY_HEAP_BYTES).expect("writer default"),
    };
    for (i, text) in docs.iter().enumerate() {
        writer
            .add_document(doc!(
                id_field => i as u64,
                title_field => text.as_str(),
            ))
            .expect("add_document");
    }
    writer.commit().expect("commit");
    TantivyHandles { index, title_field }
}

/// Top-k from Tantivy as `(internal_doc_id, score)` — no stored-doc
/// fetch. Same shape as `FtsReader::search` returns.
fn tantivy_search_scored(handles: &TantivyHandles, q: &dyn Query, k: usize) -> Vec<(u32, f32)> {
    let reader = handles.index.reader().expect("reader");
    let searcher = reader.searcher();
    let top = searcher
        .search(q, &TopDocs::with_limit(k).order_by_score())
        .expect("search");
    top.into_iter()
        .map(|(score, addr)| (addr.doc_id, score))
        .collect()
}

/// Convenience: top-k doc_ids only.
fn tantivy_search_ids(handles: &TantivyHandles, q: &dyn Query, k: usize) -> Vec<u32> {
    tantivy_search_scored(handles, q, k)
        .into_iter()
        .map(|(d, _)| d)
        .collect()
}

// ─── Correctness ──────────────────────────────────────────────────────

/// Self-consistency sanity on an infino blob: a known df=1 token
/// returns exactly one hit at the matching doc_id; a known
/// Zipfian-common term fills top-10 in descending-score order.
fn assert_infino_self_consistent(reader: &FtsReader) {
    let hits = reader
        .search("title", &["doc0500000"], 10, BoolMode::Or)
        .expect("search df=1");
    assert_eq!(hits.len(), 1, "df=1 term should return exactly one hit");
    assert_eq!(hits[0].0, 500_000, "doc0500000 should match doc_id 500000");

    let hits = reader
        .search("title", &["term00001"], 10, BoolMode::Or)
        .expect("search common");
    assert_eq!(hits.len(), 10, "common term should fill top-10");
    for w in hits.windows(2) {
        assert!(
            w[0].1 >= w[1].1,
            "results must be sorted by score desc; got {} then {}",
            w[0].1,
            w[1].1
        );
    }
}

/// BMW correctness oracle on infino: for each query, compare BMW's
/// top-10 against an effectively-brute-force ranking from the same
/// engine. Strong check — catches BMW skip bugs, BMM partition bugs,
/// and posting-decode bugs that affect ranking, all without needing
/// Tantivy as oracle.
///
/// How it works: calling `search(... k=usize::MAX, BoolMode::Or)`
/// makes BMW's pruning never fire (the heap never threshold-tightens
/// because k > df), so the result is the brute-force BM25 ranking.
/// We then sort + truncate to top-10 and compare position-by-position.
///
/// Why score equality, not doc_id equality: at score ties (which are
/// common at the top-K boundary on Zipfian corpora — many docs with
/// `(tf=1, dl≈avgdl)` land at identical BM25 scores), BMW's heap
/// keeps the first-arrived doc (strict-greater eviction) while
/// brute-force sort breaks ties by smallest doc_id. So the two paths
/// can pick **different** tied docs at the same position — both are
/// correct top-K results, just different choices from the tied set.
///
/// Comparing **scores** sidesteps this: if BMW's k-th score equals
/// brute-force's k-th score for every k in [0, 10), then BMW didn't
/// drop any real top-K candidate (pruning logic is sound). Doc_id
/// selection within tied pools is an implementation detail.
fn assert_bmw_matches_brute_force(reader: &FtsReader) -> usize {
    let battery: &[(&str, &[&str])] = &[
        ("single_rare", &["term09999"]),
        ("single_common", &["term00001"]),
        ("two_term_or", &["term00001", "term00050"]),
        ("three_wide", &["term00001", "term00050", "term00100"]),
        ("three_similar", &["term00050", "term00051", "term00052"]),
        (
            "five_term",
            &[
                "term00050", "term00051", "term00052", "term00053", "term00054",
            ],
        ),
    ];
    // BM25 scores are computed from the same (tf, dl, avgdl, idf)
    // values on both paths; modulo float-summation order differences
    // for multi-term queries, scores should be bit-identical. 1e-4
    // gives margin for any operation-order wobble on multi-term sums.
    const SCORE_EPSILON: f32 = 1e-4;

    for (label, terms) in battery {
        let bmw_top10: Vec<(u32, f32)> = reader
            .search("title", terms, 10, BoolMode::Or)
            .expect("bmw search");
        let mut brute_full = reader
            .search("title", terms, usize::MAX, BoolMode::Or)
            .expect("brute-force search");
        brute_full.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        let brute_top10: Vec<(u32, f32)> = brute_full.into_iter().take(10).collect();

        assert_eq!(
            bmw_top10.len(),
            brute_top10.len(),
            "result lengths must match on {label}: BMW {} vs brute {}",
            bmw_top10.len(),
            brute_top10.len()
        );
        for i in 0..bmw_top10.len() {
            let (bmw_doc, bmw_score) = bmw_top10[i];
            let (brute_doc, brute_score) = brute_top10[i];
            let diff = (bmw_score - brute_score).abs();
            if diff > SCORE_EPSILON {
                let bmw_seq: Vec<f32> = bmw_top10.iter().map(|(_, s)| *s).collect();
                let brute_seq: Vec<f32> = brute_top10.iter().map(|(_, s)| *s).collect();
                panic!(
                    "BMW vs brute-force score divergence at position {i} on {label} ({terms:?}):\n  \
                     BMW score = {bmw_score} (doc {bmw_doc})\n  \
                     brute score = {brute_score} (doc {brute_doc})\n  \
                     diff = {diff} > epsilon {SCORE_EPSILON}\n  \
                     BMW scores  : {bmw_seq:?}\n  \
                     brute scores: {brute_seq:?}"
                );
            }
        }
    }
    battery.len()
}

/// Cross-engine correctness check at the bench scale. df=1
/// `doc0500000` has exactly one true positive by construction;
/// both engines must return that one doc. Strict top-K agreement
/// at Zipfian scale is unreliable (deeply tied score pools); the
/// strict oracle lives at smaller corpora — see
/// `tests/superfile/fts/against_tantivy.rs` (60-doc planted) and
/// `benches/scale/fts_recall.rs` (20K-doc strict).
fn assert_cross_engine_df1_match(reader: &FtsReader, t: &TantivyHandles) -> usize {
    let parser = QueryParser::for_index(&t.index, vec![t.title_field]);
    let qstr = "doc0500000";
    let terms: &[&str] = &["doc0500000"];

    let inf_hits: Vec<(u32, f32)> = reader
        .search("title", terms, 10, BoolMode::Or)
        .expect("infino search");
    let parsed = parser.parse_query(qstr).expect("parse_query");
    let tan_ids: Vec<u32> = tantivy_search_ids(t, parsed.as_ref(), 10);

    assert_eq!(inf_hits.len(), 1, "df=1 expected exactly one infino hit");
    assert_eq!(tan_ids.len(), 1, "df=1 expected exactly one tantivy hit");
    assert_eq!(
        inf_hits[0].0, tan_ids[0],
        "df=1 cross-engine doc_id mismatch on {qstr:?}"
    );
    assert!(inf_hits[0].1 > 0.0, "df=1 score must be positive");
    1
}

// ─── Bench helpers ────────────────────────────────────────────────────

/// Add a head-to-head pair (infino + tantivy) for a query shape.
fn bench_pair(
    g: &mut BenchmarkGroup<WallTime>,
    name: &str,
    r: &FtsReader,
    t: &TantivyHandles,
    infino_terms: &'static [&'static str],
    tantivy_query: &dyn Query,
) {
    g.bench_function(format!("{name}_infino_top10"), |b| {
        b.iter(|| {
            let hits = r
                .search(
                    black_box("title"),
                    black_box(infino_terms),
                    black_box(10),
                    BoolMode::Or,
                )
                .expect("infino search");
            black_box(hits)
        });
    });
    g.bench_function(format!("{name}_tantivy_top10"), |b| {
        b.iter(|| {
            let hits = tantivy_search_scored(t, tantivy_query, black_box(10));
            black_box(hits)
        });
    });
}

/// Add an infino-only per-algorithm probe pair (WAND+BMW vs
/// MaxScore+BMM) for one query shape. Bypasses the dispatcher; lets
/// us measure each algorithm directly so the heuristic's threshold
/// can be validated against ground truth.
fn bench_per_algo_probe(
    g: &mut BenchmarkGroup<WallTime>,
    name: &str,
    r: &FtsReader,
    terms: &'static [&'static str],
) {
    g.bench_function(format!("{name}_wand_top10"), |b| {
        b.iter(|| {
            let hits = r
                .search_with_algo_for_bench(
                    black_box("title"),
                    black_box(terms),
                    black_box(10),
                    OrAlgo::WandBmw,
                )
                .expect("WAND+BMW search");
            black_box(hits)
        });
    });
    g.bench_function(format!("{name}_bmm_top10"), |b| {
        b.iter(|| {
            let hits = r
                .search_with_algo_for_bench(
                    black_box("title"),
                    black_box(terms),
                    black_box(10),
                    OrAlgo::Bmm,
                )
                .expect("MaxScore+BMM search");
            black_box(hits)
        });
    });
}

// ─── Bench entry ──────────────────────────────────────────────────────

/// FTS superfile bench: runs correctness gates once, then registers
/// the ingest sub-group (`superfile_fts_build`) and the search sub-group
/// (`superfile_fts_search`, with head-to-head pairs + per-algo probes).
fn bench(c: &mut Criterion) {
    // ---- Correctness phase (runs regardless of criterion filter) ---
    eprintln!("[fts] correctness: building infino + Tantivy ({N_DOCS} docs)...");
    let r = infino_reader();
    let t = tantivy_handles();
    assert_infino_self_consistent(&r);
    let n_bmw = assert_bmw_matches_brute_force(&r);
    let n_df1 = assert_cross_engine_df1_match(&r, t);
    eprintln!(
        "[fts] correctness OK: infino self-consistent + {n_bmw} queries BMW==brute-force + \
         {n_df1} df=1 cross-engine match (strict top-K oracle is in \
         tests/superfile/fts/against_tantivy.rs + benches/scale/fts_recall)"
    );

    // ---- Ingest sub-bench (group: superfile_fts_build) -----------------------
    {
        let n = N_DOCS;
        let docs_for_ingest = docs();
        let mut g = c.benchmark_group("superfile_fts_build");
        g.sample_size(10);
        g.throughput(Throughput::Elements(n as u64));

        g.bench_function(format!("infino_1thread_{n}docs"), |b| {
            b.iter_with_large_drop(|| build_infino_blob_1thread(black_box(docs_for_ingest)));
        });
        g.bench_function(format!("infino_rayon_default_threads_{n}docs"), |b| {
            b.iter_with_large_drop(|| build_infino_blobs_rayon(black_box(docs_for_ingest)));
        });
        g.bench_function(format!("tantivy_1thread_{n}docs"), |b| {
            b.iter_with_large_drop(|| {
                build_tantivy(black_box(docs_for_ingest), TantivyThreads::Single)
            });
        });
        g.bench_function(format!("tantivy_default_threads_{n}docs"), |b| {
            b.iter_with_large_drop(|| {
                build_tantivy(black_box(docs_for_ingest), TantivyThreads::Default)
            });
        });
        g.finish();

        emit_ingest_markdown();
    }

    // ---- Search sub-bench (group: superfile_fts_search) ----------------------
    {
        let parser = QueryParser::for_index(&t.index, vec![t.title_field]);
        let q_single_rare = parser.parse_query("term09999").expect("parse");
        let q_single_df1 = parser.parse_query("doc0500000").expect("parse");
        let q_single_common = parser.parse_query("term00001").expect("parse");
        let q_two_term = parser.parse_query("term00001 term00050").expect("parse");
        let q_three_wide = parser
            .parse_query("term00001 term00050 term00100")
            .expect("parse");
        let q_three_similar = parser
            .parse_query("term00050 term00051 term00052")
            .expect("parse");
        let q_five = parser
            .parse_query("term00050 term00051 term00052 term00053 term00054")
            .expect("parse");

        let mut g = c.benchmark_group("superfile_fts_search");

        bench_pair(&mut g, "single_rare", &r, t, &["term09999"], q_single_rare.as_ref());
        bench_pair(&mut g, "single_df1", &r, t, &["doc0500000"], q_single_df1.as_ref());
        bench_pair(&mut g, "single_common", &r, t, &["term00001"], q_single_common.as_ref());
        bench_pair(&mut g, "two_term_or", &r, t, &["term00001", "term00050"], q_two_term.as_ref());
        bench_pair(
            &mut g,
            "three_wide",
            &r,
            t,
            &["term00001", "term00050", "term00100"],
            q_three_wide.as_ref(),
        );
        bench_pair(
            &mut g,
            "three_similar",
            &r,
            t,
            &["term00050", "term00051", "term00052"],
            q_three_similar.as_ref(),
        );
        bench_pair(
            &mut g,
            "five_term",
            &r,
            t,
            &[
                "term00050", "term00051", "term00052", "term00053", "term00054",
            ],
            q_five.as_ref(),
        );

        // Per-algo probes (infino-only)
        bench_per_algo_probe(
            &mut g,
            "wide_3",
            &r,
            &["term00001", "term00050", "term00100"],
        );
        bench_per_algo_probe(
            &mut g,
            "similar_3",
            &r,
            &["term00050", "term00051", "term00052"],
        );
        bench_per_algo_probe(
            &mut g,
            "similar_5",
            &r,
            &[
                "term00050", "term00051", "term00052", "term00053", "term00054",
            ],
        );

        g.finish();

        emit_search_markdown();
    }
}

// ─── Markdown summary emitters ────────────────────────────────────────

fn emit_ingest_markdown() {
    use crate::markdown::{MarkdownSection, fmt_throughput, fmt_time, fmt_winner, read_mean_ns};

    let mut body = String::new();
    body.push_str(&format!(
        "### Superfile FTS — ingest ({N_DOCS} docs, Zipfian, 200 tokens/doc, 10K vocab)\n\n"
    ));
    body.push_str("| Engine                       | Time       | Throughput | vs Tantivy        |\n");
    body.push_str("|------------------------------|------------|------------|-------------------|\n");

    let group = "superfile_fts_build";
    let infino_1t = read_mean_ns(group, &format!("infino_1thread_{N_DOCS}docs"));
    let infino_rayon = read_mean_ns(group, &format!("infino_rayon_default_threads_{N_DOCS}docs"));
    let tantivy_1t = read_mean_ns(group, &format!("tantivy_1thread_{N_DOCS}docs"));
    let tantivy_def = read_mean_ns(group, &format!("tantivy_default_threads_{N_DOCS}docs"));

    let row = |label: &str, ns: Option<f64>, baseline: Option<f64>, is_baseline: bool| -> String {
        let time = ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let thrpt = ns
            .map(|n| fmt_throughput((N_DOCS as f64) / (n / 1e9)))
            .unwrap_or_else(|| "—".into());
        // Comparison shown only on non-baseline rows. Engine names
        // (not bench-id strings) so the column reads cleanly.
        let cmp = if is_baseline {
            "—".to_string()
        } else {
            fmt_winner("infino", ns, "tantivy", baseline)
        };
        format!("| {label:28} | {time:10} | {thrpt:10} | {cmp:17} |\n")
    };

    body.push_str(&row("infino_1thread", infino_1t, tantivy_1t, false));
    body.push_str(&row("tantivy_1thread", tantivy_1t, tantivy_1t, true));
    body.push_str(&row(
        "infino_rayon_default_threads",
        infino_rayon,
        tantivy_def,
        false,
    ));
    body.push_str(&row("tantivy_default_threads", tantivy_def, tantivy_def, true));

    crate::markdown::emit(&MarkdownSection {
        anchor_id: "bench/fts/superfile/ingest".into(),
        body,
    });
}

fn emit_search_markdown() {
    use crate::markdown::{MarkdownSection, fmt_time, fmt_winner, read_mean_ns};

    let mut body = String::new();
    body.push_str(&format!("### Superfile FTS — search ({N_DOCS} docs)\n\n"));
    body.push_str("| Query          | infino     | Tantivy    | Winner                |\n");
    body.push_str("|----------------|------------|------------|-----------------------|\n");

    let group = "superfile_fts_search";
    let queries = [
        "single_rare",
        "single_df1",
        "single_common",
        "two_term_or",
        "three_wide",
        "three_similar",
        "five_term",
    ];
    for q in queries {
        let inf = read_mean_ns(group, &format!("{q}_infino_top10"));
        let tan = read_mean_ns(group, &format!("{q}_tantivy_top10"));
        let inf_s = inf.map(fmt_time).unwrap_or_else(|| "—".into());
        let tan_s = tan.map(fmt_time).unwrap_or_else(|| "—".into());
        let w = fmt_winner("infino", inf, "tantivy", tan);
        body.push_str(&format!(
            "| {q:14} | {inf_s:10} | {tan_s:10} | {w:21} |\n"
        ));
    }

    body.push_str("\n");
    body.push_str("**Per-algorithm probes** (infino-only, WAND+BMW vs MaxScore+BMM):\n\n");
    body.push_str("| Shape         | WAND+BMW   | MaxScore+BMM | Winner                |\n");
    body.push_str("|---------------|------------|--------------|-----------------------|\n");
    for shape in ["wide_3", "similar_3", "similar_5"] {
        let wand = read_mean_ns(group, &format!("{shape}_wand_top10"));
        let bmm = read_mean_ns(group, &format!("{shape}_bmm_top10"));
        let wand_s = wand.map(fmt_time).unwrap_or_else(|| "—".into());
        let bmm_s = bmm.map(fmt_time).unwrap_or_else(|| "—".into());
        let w = fmt_winner("WAND+BMW", wand, "MaxScore+BMM", bmm);
        body.push_str(&format!(
            "| {shape:13} | {wand_s:10} | {bmm_s:12} | {w:21} |\n"
        ));
    }

    crate::markdown::emit(&MarkdownSection {
        anchor_id: "bench/fts/superfile/search".into(),
        body,
    });
}

criterion_group!(benches, bench);
