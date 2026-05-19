//! Single-binary FTS bench for the supertable layer:
//!
//!   ingest head-to-head infino vs Tantivy (10M docs)
//! + 7-query search head-to-head (parallel mode — both engines on
//!   a shared `num_cpus` rayon pool)
//! + correctness gates (df=1 cross-engine match, infino self-
//!   consistency)
//!
//! Multi-segment shape: both engines shard the same 10M-doc Zipfian
//! corpus into [`SEGMENTS`] commits. Tantivy's auto-merge is disabled
//! (`NoMergePolicy`) so per-segment IDF stays apples-to-apples with
//! the supertable's per-superfile scoring. Infino's `commit()`
//! row-shards into `min(writer_pool.threads, total_rows)` superfiles
//! — the writer-pool size doubles as the output-cardinality dial
//! (auto = `cpus/2` by default; override with
//! `INFINO_SUPERTABLE__WRITER_THREADS=N`).
//!
//! ## Search threading
//!
//! Both engines share the same `num_cpus`-sized rayon pool so
//! neither gets a CPU budget the other doesn't. Tantivy gets manual
//! cross-segment parallelism via `par_iter` over `SegmentReader`s +
//! `weight.for_each_pruning` (BMW), matching what infino's
//! supertable reader pool does natively.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench fts -- supertable_fts                      # both supertable groups
//! cargo bench --bench fts -- supertable_fts_build                # ingest only
//! cargo bench --bench fts -- supertable_fts_search                # search only
//! INFINO_SUPERTABLE__WRITER_THREADS=32 cargo bench --bench fts -- supertable_fts_build
//!     # ingest at 32 superfiles (matches Tantivy's 32-segment effective output)
//! ```

use std::hint::black_box;
use std::sync::{Arc, OnceLock};

use arrow_array::{LargeStringArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use criterion::{Criterion, Throughput, criterion_group};
use infino::superfile::builder::FtsConfig;
use infino::superfile::fts::reader::BoolMode;
use infino::superfile::fts::tokenize::Tokenizer;
use infino::supertable::{Supertable, SupertableOptions};
use infino::test_helpers::default_tokenizer;
use rayon::ThreadPool;
use rayon::prelude::*;
use tantivy::Index;
use tantivy::Term;
use tantivy::collector::TopDocs;
use tantivy::doc;
use tantivy::indexer::NoMergePolicy;
use tantivy::query::{
    BooleanQuery, EnableScoring, Occur, Query, QueryParser, TermQuery,
};
use tantivy::schema::{
    INDEXED, IndexRecordOption, STORED, Schema as TSchema, TextFieldIndexing, TextOptions,
};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};

// ─── Constants ────────────────────────────────────────────────────────

/// Doc count for every FTS-supertable bench. Pinned to 10M — the
/// supertable shape is "scale-out via superfiles," so the right
/// scale to measure is well above the single-superfile floor (1M).
const N_DOCS: usize = 10_000_000;

/// Input chunk count for both engines. Drives Tantivy's per-commit
/// cycle (it emits N segments per commit at default thread count);
/// infino's output superfile count is governed by writer_pool
/// threads, not by this knob, so this only sets the
/// `append()`-batching shape.
const SEGMENTS: usize = 4;

const TOP_K: usize = 10;

/// Heap budget for the Tantivy IndexWriter. At 10M-doc scale the
/// inverted-index posting volume is large; well above what fits
/// without spilling mid-commit.
const TANTIVY_HEAP_BYTES: usize = 2_000_000_000;

// ─── Fixtures (built once, reused across criterion samples) ────────────

static DOCS: OnceLock<Vec<String>> = OnceLock::new();
static INFINO_PARALLEL: OnceLock<Supertable> = OnceLock::new();
static TANTIVY: OnceLock<TantivyHandles> = OnceLock::new();

fn docs() -> &'static [String] {
    DOCS.get_or_init(|| crate::corpus::generate_text_corpus(N_DOCS, 1))
        .as_slice()
}

fn infino_parallel() -> &'static Supertable {
    INFINO_PARALLEL.get_or_init(|| build_supertable_infino(docs(), parallel_pool()))
}

fn tantivy_handles() -> &'static TantivyHandles {
    TANTIVY.get_or_init(|| build_supertable_tantivy(docs()))
}

// ─── Shared rayon pool ────────────────────────────────────────────────

/// num_cpus-sized pool **shared** between infino (as its reader
/// pool) and the parallel-mode Tantivy helper (as the pool that
/// `par_iter` over `SegmentReader`s runs on). Sharing ensures both
/// engines compete for the same N threads.
fn parallel_pool() -> Arc<ThreadPool> {
    static POOL: OnceLock<Arc<ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(num_cpus::get().max(1))
                .thread_name(|i| format!("supertable-fts-bench-par-{i}"))
                .build()
                .expect("parallel pool"),
        )
    })
    .clone()
}

// ─── Builders — infino ────────────────────────────────────────────────

fn schema_id_title() -> Arc<Schema> {
    Arc::new(Schema::new(vec![Field::new(
        "title",
        DataType::LargeUtf8,
        false,
    )]))
}

fn supertable_options(reader_pool: Arc<ThreadPool>) -> SupertableOptions {
    let tk: Arc<dyn Tokenizer> = default_tokenizer();
    SupertableOptions::new(
        schema_id_title(),
        vec![FtsConfig {
            column: "title".into(),
        }],
        vec![],
        Some(tk),
    )
    .expect("opts")
    // Writer pool intentionally left at the SupertableOptions auto
    // default (= cpus/2). Override via
    // `INFINO_SUPERTABLE__WRITER_THREADS=N` if you want to control
    // the output superfile cardinality from the env.
    .with_reader_pool(reader_pool)
    // Bench raises the commit-threshold sky-high so `append()`
    // doesn't auto-flush mid-build. With a 1 GiB default at
    // ~4.5 GB / chunk, default options would auto-commit after
    // every single append — and the supertable's `commit()` runs
    // per-shard work in parallel only **within** a commit. By
    // buffering all chunks before the explicit final commit
    // below, we let `commit()` row-shard across all writer-pool
    // threads in one go.
    .with_commit_threshold_size_mb(0)
}

/// Build an FTS-only supertable from `docs`.
///
/// **Append-many-then-commit-once pattern**: each chunk is appended
/// to the writer's buffer (auto-flush disabled via
/// `commit_threshold_size_mb=0`); a single `commit()` at the end
/// drains the buffer and row-shards across the writer pool. The
/// number of output superfiles is `min(writer_pool.threads,
/// total_rows)` — driven by infino's commit-time row-sharding, not
/// by the number of `append()` calls or the [`SEGMENTS`] constant.
fn build_supertable_infino(docs: &[String], reader_pool: Arc<ThreadPool>) -> Supertable {
    let st = Supertable::create(supertable_options(reader_pool));
    let mut w = st.writer().expect("writer");
    let chunk_size = docs.len().div_ceil(SEGMENTS);
    for chunk in docs.chunks(chunk_size) {
        let titles = LargeStringArray::from(chunk.iter().map(String::as_str).collect::<Vec<_>>());
        let batch = RecordBatch::try_new(schema_id_title(), vec![Arc::new(titles)])
            .expect("batch");
        w.append(&batch).expect("append");
    }
    w.commit().expect("commit");
    drop(w);
    st
}

// ─── Builders — Tantivy ───────────────────────────────────────────────

struct TantivyHandles {
    index: Index,
    title_field: tantivy::schema::Field,
}

/// Build a Tantivy index with `NoMergePolicy` so segments stay
/// at the per-commit count. `WithFreqs` (no positions) matches
/// infino's `(doc_id, tf)`-only posting layout.
fn build_supertable_tantivy(docs: &[String]) -> TantivyHandles {
    let mut sb = TSchema::builder();
    let id_field = sb.add_u64_field("doc_id", INDEXED | STORED);
    let title_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(IndexRecordOption::WithFreqs),
    );
    let title_field = sb.add_text_field("title", title_opts);
    let schema = sb.build();
    let index = Index::builder()
        .schema(schema)
        .create_in_ram()
        .expect("create_in_ram");
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build();
    index.tokenizers().register("default", analyzer);

    let mut writer = index.writer(TANTIVY_HEAP_BYTES).expect("writer");
    writer.set_merge_policy(Box::new(NoMergePolicy));

    let chunk_size = docs.len().div_ceil(SEGMENTS);
    for (chunk_idx, chunk) in docs.chunks(chunk_size).enumerate() {
        let start = chunk_idx * chunk_size;
        for (i, t) in chunk.iter().enumerate() {
            writer
                .add_document(doc!(
                    id_field => (start + i) as u64,
                    title_field => t.as_str(),
                ))
                .expect("add_document");
        }
        writer.commit().expect("commit");
    }
    drop(writer);

    TantivyHandles { index, title_field }
}

/// Tantivy's default-API search — `Searcher::search` walks all
/// segments **sequentially**. This is what a Tantivy user gets out
/// of the box.
fn tantivy_search_serial(
    handles: &TantivyHandles,
    q: &dyn Query,
    k: usize,
) -> Vec<(u32, f32)> {
    let reader = handles.index.reader().expect("reader");
    let searcher = reader.searcher();
    let top = searcher
        .search(q, &TopDocs::with_limit(k).order_by_score())
        .expect("search");
    top.into_iter()
        .map(|(score, addr)| (addr.doc_id, score))
        .collect()
}

/// Tantivy + manual cross-segment parallelism using rayon
/// `par_iter` over `SegmentReader`s on the supplied pool. Uses
/// `weight.for_each_pruning` to match what `TopDocs::order_by_score`
/// does serially — BooleanWeight overrides this with `block_wand`
/// (BMW), the same skip-pruning class infino uses. Without this
/// match, the parallel helper would use the default `for_each`
/// (exhaustive walk over the union), which on rare-term unions
/// accidentally beats BMW's per-block bookkeeping overhead — an
/// algorithm asymmetry that flatters Tantivy in heavy-query parallel
/// timings vs infino's BMW path.
fn tantivy_search_parallel(
    handles: &TantivyHandles,
    q: &dyn Query,
    k: usize,
    pool: &ThreadPool,
) -> Vec<(u32, f32)> {
    use std::cmp::Ordering;
    use std::collections::BinaryHeap;

    let reader = handles.index.reader().expect("reader");
    let searcher = reader.searcher();
    let weight = q
        .weight(EnableScoring::enabled_from_searcher(&searcher))
        .expect("weight");

    #[derive(Clone, Copy)]
    struct HeapEntry(f32, u32);
    impl PartialEq for HeapEntry {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0 && self.1 == other.1
        }
    }
    impl Eq for HeapEntry {}
    impl PartialOrd for HeapEntry {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for HeapEntry {
        fn cmp(&self, other: &Self) -> Ordering {
            other
                .0
                .partial_cmp(&self.0)
                .unwrap_or(Ordering::Equal)
                .then_with(|| other.1.cmp(&self.1))
        }
    }

    let per_seg: Vec<Vec<(f32, u32)>> = pool.install(|| {
        searcher
            .segment_readers()
            .par_iter()
            .map(|seg_reader| {
                let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::with_capacity(k);
                weight
                    .for_each_pruning(f32::MIN, seg_reader, &mut |doc, score| {
                        if heap.len() < k {
                            heap.push(HeapEntry(score, doc));
                        } else if let Some(top) = heap.peek() {
                            if score > top.0 {
                                heap.pop();
                                heap.push(HeapEntry(score, doc));
                            }
                        }
                        heap.peek().map(|h| h.0).unwrap_or(f32::MIN)
                    })
                    .expect("for_each_pruning");
                heap.into_iter().map(|HeapEntry(s, d)| (s, d)).collect()
            })
            .collect()
    });

    let mut all: Vec<(f32, u32)> = per_seg.into_iter().flatten().collect();
    all.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    all.truncate(k);
    all.into_iter().map(|(score, doc)| (doc, score)).collect()
}

// ─── Correctness ──────────────────────────────────────────────────────

/// Self-consistency on the built supertable: the corpus's df=1
/// identifier `doc<id:07>` returns exactly one hit; a Zipfian-common
/// term fills top-10 in score-desc order.
fn assert_infino_self_consistent(st: &Supertable) {
    let r = st.reader();
    let probe_doc_id = (N_DOCS / 2) as u32;
    let probe_token = format!("doc{probe_doc_id:07}");
    let hits = r
        .bm25_search("title", &probe_token, 10, BoolMode::Or)
        .expect("bm25");
    assert_eq!(
        hits.len(),
        1,
        "df=1 token {probe_token:?} should return exactly one hit; got {}",
        hits.len()
    );
    assert!(
        hits[0].score > 0.0,
        "df=1 score must be positive; got {}",
        hits[0].score
    );

    let hits = r
        .bm25_search("title", "term00001", 10, BoolMode::Or)
        .expect("bm25");
    assert_eq!(hits.len(), 10, "common term should fill top-10");
    for w in hits.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "results must be sorted by score desc; got {} then {}",
            w[0].score,
            w[1].score
        );
    }
}

/// Cross-engine sanity at the supertable scale: the corpus's per-doc
/// unique identifier has df=1 across all superfiles. Both engines
/// must return exactly one hit. Strict top-K agreement at 10M-doc
/// Zipfian scale is unreliable (deeply tied score pools); strict
/// oracle correctness lives in
/// `tests/supertable/query/against_tantivy.rs` (planted-truth) and
/// `benches/scale/fts_recall.rs` (20K-doc strict).
fn assert_cross_engine_df1_match(st: &Supertable, t: &TantivyHandles) -> usize {
    let r = st.reader();
    let probe_doc_id = (N_DOCS / 2) as u32;
    let probe_token = format!("doc{probe_doc_id:07}");

    let inf_hits = r
        .bm25_search("title", &probe_token, 10, BoolMode::Or)
        .expect("infino bm25");
    let parser = QueryParser::for_index(&t.index, vec![t.title_field]);
    let parsed = parser.parse_query(&probe_token).expect("parse");
    let tan_ids: Vec<u32> = tantivy_search_serial(t, parsed.as_ref(), 10)
        .into_iter()
        .map(|(doc, _)| doc)
        .collect();

    assert_eq!(inf_hits.len(), 1, "df=1 infino: expected one hit");
    assert_eq!(tan_ids.len(), 1, "df=1 tantivy: expected one hit");
    assert!(
        inf_hits[0].score > 0.0,
        "df=1 infino score must be positive"
    );
    1
}

// ─── Query battery (shared between serial + parallel modes) ───────────

struct Battery {
    q_single_rare: Box<dyn Query>,
    q_single_common: Box<dyn Query>,
    q_two: Box<dyn Query>,
    q_three_wide: Box<dyn Query>,
    q_three_similar: Box<dyn Query>,
    q_five: Box<dyn Query>,
    q_prefix: BooleanQuery,
}

fn battery(t: &TantivyHandles) -> Battery {
    let parser = QueryParser::for_index(&t.index, vec![t.title_field]);
    let q_single_rare = parser.parse_query("term09999").expect("parse");
    let q_single_common = parser.parse_query("term00001").expect("parse");
    let q_two = parser.parse_query("term00001 term00050").expect("parse");
    let q_three_wide = parser
        .parse_query("term00001 term00050 term00100")
        .expect("parse");
    let q_three_similar = parser
        .parse_query("term00050 term00051 term00052")
        .expect("parse");
    let q_five = parser
        .parse_query("term00050 term00051 term00052 term00053 term00054")
        .expect("parse");

    // Manual prefix-expansion: `term0009*` → term00090..term00099.
    // Tantivy 0.26's QueryParser doesn't expand inline wildcards.
    let prefix_terms: Vec<String> = (90..100).map(|i| format!("term{i:05}")).collect();
    let prefix_subqueries: Vec<(Occur, Box<dyn Query>)> = prefix_terms
        .iter()
        .map(|term_str| {
            let term = Term::from_field_text(t.title_field, term_str);
            let q: Box<dyn Query> = Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs));
            (Occur::Should, q)
        })
        .collect();
    let q_prefix = BooleanQuery::new(prefix_subqueries);

    Battery {
        q_single_rare,
        q_single_common,
        q_two,
        q_three_wide,
        q_three_similar,
        q_five,
        q_prefix,
    }
}

// ─── Bench: ingest (group: supertable_fts_build) ──────────────────────

fn bench_ingest(c: &mut Criterion) {
    // ---- Correctness phase ----------------------------------------
    eprintln!(
        "[supertable_fts_build] correctness: building infino + Tantivy ({N_DOCS} docs)..."
    );
    let infino = build_supertable_infino(docs(), parallel_pool());
    assert_infino_self_consistent(&infino);
    let tantivy = tantivy_handles();
    let n_df1 = assert_cross_engine_df1_match(&infino, tantivy);
    eprintln!(
        "[supertable_fts_build] correctness OK: infino self-consistent + {n_df1} df=1 \
         cross-engine match (strict top-K oracle is in tests/supertable/query/against_tantivy.rs)"
    );
    drop(infino);

    // ---- Timing phase ---------------------------------------------
    let mut g = c.benchmark_group("supertable_fts_build");
    g.sample_size(10);
    g.throughput(Throughput::Elements(N_DOCS as u64));

    g.bench_function("infino_auto_writer_pool", |b| {
        b.iter_with_large_drop(|| build_supertable_infino(black_box(docs()), parallel_pool()));
    });

    g.bench_function("tantivy_default_threads", |b| {
        b.iter_with_large_drop(|| build_supertable_tantivy(black_box(docs())));
    });

    g.finish();

    emit_ingest_markdown();
}

// ─── Bench: search-parallel (group: supertable_fts_search) ───

fn bench_search(c: &mut Criterion) {
    let st = infino_parallel();
    let t = tantivy_handles();
    let pool = parallel_pool();

    eprintln!("[supertable_fts_search] correctness check...");
    assert_infino_self_consistent(st);
    let n_df1 = assert_cross_engine_df1_match(st, t);
    eprintln!(
        "[supertable_fts_search] correctness OK: infino self-consistent + {n_df1} df=1 \
         cross-engine match (rayon pool: {} threads)",
        pool.current_num_threads()
    );

    let r = st.reader();
    let qs = battery(t);

    let mut g = c.benchmark_group("supertable_fts_search");
    g.sample_size(10);

    macro_rules! pair {
        ($name:literal, $infino_query:expr, $tantivy_query:expr) => {
            g.bench_function(concat!($name, "_supertable_top10"), |b| {
                b.iter(|| {
                    let hits = r
                        .bm25_search(
                            black_box("title"),
                            black_box($infino_query),
                            TOP_K,
                            BoolMode::Or,
                        )
                        .expect("bm25");
                    black_box(hits)
                });
            });
            g.bench_function(concat!($name, "_tantivy_top10"), |b| {
                b.iter(|| {
                    let hits = tantivy_search_parallel(t, $tantivy_query, TOP_K, &pool);
                    black_box(hits)
                });
            });
        };
    }

    pair!("single_rare", "term09999", qs.q_single_rare.as_ref());
    pair!("single_common", "term00001", qs.q_single_common.as_ref());
    pair!("two_term_or", "term00001 term00050", qs.q_two.as_ref());
    pair!(
        "three_wide",
        "term00001 term00050 term00100",
        qs.q_three_wide.as_ref()
    );
    pair!(
        "three_similar",
        "term00050 term00051 term00052",
        qs.q_three_similar.as_ref()
    );
    pair!(
        "five_term",
        "term00050 term00051 term00052 term00053 term00054",
        qs.q_five.as_ref()
    );

    g.bench_function("prefix_supertable_top10", |b| {
        b.iter(|| {
            let hits = r
                .bm25_search_prefix(black_box("title"), black_box("term0009"), TOP_K)
                .expect("bm25_prefix");
            black_box(hits)
        });
    });
    g.bench_function("prefix_tantivy_top10", |b| {
        b.iter(|| {
            let hits = tantivy_search_parallel(t, &qs.q_prefix, TOP_K, &pool);
            black_box(hits)
        });
    });

    g.finish();

    emit_search_markdown();
}

// ─── Markdown summary emitters ────────────────────────────────────────

fn emit_ingest_markdown() {
    use crate::markdown::{MarkdownSection, fmt_throughput, fmt_time, fmt_winner, read_mean_ns};

    let group = "supertable_fts_build";
    let infino_ns = read_mean_ns(group, "infino_auto_writer_pool");
    let tantivy_ns = read_mean_ns(group, "tantivy_default_threads");

    let mut body = String::new();
    body.push_str(&format!(
        "### Supertable FTS — ingest ({N_DOCS} docs, Zipfian, 200 tokens/doc, 10K vocab)\n\n"
    ));
    body.push_str(
        "| Engine                  | Time       | Throughput | vs Tantivy        |\n",
    );
    body.push_str(
        "|-------------------------|------------|------------|-------------------|\n",
    );
    for (label, ns, baseline_ns, is_baseline) in [
        ("infino_auto_writer_pool", infino_ns, tantivy_ns, false),
        ("tantivy_default_threads", tantivy_ns, tantivy_ns, true),
    ] {
        let time = ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let thrpt = ns
            .map(|n| fmt_throughput((N_DOCS as f64) / (n / 1e9)))
            .unwrap_or_else(|| "—".into());
        let cmp = if is_baseline {
            "—".to_string()
        } else {
            fmt_winner("infino", ns, "tantivy", baseline_ns)
        };
        body.push_str(&format!("| {label:23} | {time:10} | {thrpt:10} | {cmp:17} |\n"));
    }
    body.push_str(
        "\n*Output cardinality: infino emits `min(writer_pool.threads, total_rows)` superfiles \
         per commit (auto = cpus/2). Tantivy emits one segment per internal worker thread per \
         commit (≈ 8 × N_chunks segments with NoMergePolicy). Override the infino writer-thread \
         count with `INFINO_SUPERTABLE__WRITER_THREADS=N` to match Tantivy's effective output \
         segment count.*\n",
    );

    crate::markdown::emit(&MarkdownSection {
        anchor_id: "bench/fts/supertable/ingest".into(),
        body,
    });
}

fn emit_search_markdown() {
    use crate::markdown::{MarkdownSection, fmt_time, fmt_winner, read_mean_ns};

    let group = "supertable_fts_search";
    let mut body = String::new();
    body.push_str(&format!("### Supertable FTS — search ({N_DOCS} docs)\n\n"));
    body.push_str("| Query          | infino     | Tantivy    | Winner                |\n");
    body.push_str("|----------------|------------|------------|-----------------------|\n");
    let queries = [
        "single_rare",
        "single_common",
        "two_term_or",
        "three_wide",
        "three_similar",
        "five_term",
        "prefix",
    ];
    for q in queries {
        let inf = read_mean_ns(group, &format!("{q}_supertable_top10"));
        let tan = read_mean_ns(group, &format!("{q}_tantivy_top10"));
        let inf_s = inf.map(fmt_time).unwrap_or_else(|| "—".into());
        let tan_s = tan.map(fmt_time).unwrap_or_else(|| "—".into());
        let w = fmt_winner("infino", inf, "tantivy", tan);
        body.push_str(&format!("| {q:14} | {inf_s:10} | {tan_s:10} | {w:21} |\n"));
    }

    crate::markdown::emit(&MarkdownSection {
        anchor_id: "bench/fts/supertable/search".into(),
        body,
    });
}

criterion_group!(benches, bench_ingest, bench_search);
