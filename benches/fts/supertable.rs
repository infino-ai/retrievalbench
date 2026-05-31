//! Tantivy head-to-head FTS bench for the supertable layer.
//!
//! Measures Tantivy only — infino's own numbers come from
//! `infino/benches` (read out of `../infino/target/criterion/...` at
//! emit time). The corpus is shared between both repos via
//! `infino::test_helpers::bench_corpus`.
//!
//! Tantivy's auto-merge is disabled (`NoMergePolicy`) so per-segment
//! IDF stays apples-to-apples with the supertable's per-superfile
//! scoring at the same chunk count.
//!
//! Both engines share the same `num_cpus`-sized rayon pool so neither
//! gets a CPU budget the other doesn't. Tantivy gets manual
//! cross-segment parallelism via `par_iter` over `SegmentReader`s +
//! `weight.for_each_pruning` (BMW), matching what infino's supertable
//! reader pool does natively.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench supertable_all -- supertable_fts           # both groups
//! cargo bench --bench supertable_all -- supertable_fts_build     # ingest only
//! cargo bench --bench supertable_all -- supertable_fts_search    # search only
//! ```

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use criterion::{Criterion, Throughput, criterion_group};
use rayon::ThreadPool;
use rayon::prelude::*;
use retrievalbench::object_store_tier::Tier;
use retrievalbench::{corpus, markdown, results, rss};
use tantivy::DocAddress;
use tantivy::Index;
use tantivy::Score;
use tantivy::Searcher;
use tantivy::Term;
use tantivy::collector::Collector;
use tantivy::collector::TopDocs;
use tantivy::doc;
use tantivy::indexer::NoMergePolicy;
use tantivy::query::Weight;
use tantivy::query::{BooleanQuery, EnableScoring, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{
    INDEXED, IndexRecordOption, STORED, Schema as TSchema, TextFieldIndexing, TextOptions,
};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};
use tempfile::TempDir;

// ─── Constants ────────────────────────────────────────────────────────

const N_DOCS: usize = corpus::SUPERTABLE_DOCS;
const SEGMENTS: usize = 4;
const TOP_K: usize = 10;
const TANTIVY_HEAP_BYTES: usize = 2_000_000_000;

// ─── Fixtures ────────────────────────────────────────────────────────

static DOCS: OnceLock<Vec<String>> = OnceLock::new();
static TANTIVY: OnceLock<TantivyHandles> = OnceLock::new();

/// Disk-backed Tantivy index (warm/cold tiers). Tantivy has no native S3
/// integration; disk mmap is the closest parity to object-store cold open.
struct DiskTantivyFixture {
    _dir: TempDir,
    title_field: tantivy::schema::Field,
}
static TANTIVY_DISK: OnceLock<DiskTantivyFixture> = OnceLock::new();

fn docs() -> &'static [String] {
    DOCS.get_or_init(|| corpus::generate_text_corpus(N_DOCS, 1))
        .as_slice()
}

fn tantivy_handles() -> &'static TantivyHandles {
    TANTIVY.get_or_init(|| build_supertable_tantivy(docs()))
}

// ─── Shared rayon pool ────────────────────────────────────────────────

/// `num_cpus`-sized pool — Tantivy's `par_iter` over SegmentReaders
/// installs on it.
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

// ─── Tantivy builder + search ─────────────────────────────────────────

struct TantivyHandles {
    index: Index,
    title_field: tantivy::schema::Field,
}

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

fn build_tantivy_on_disk(docs: &[String]) -> DiskTantivyFixture {
    let dir = TempDir::new().expect("tantivy disk tempdir");
    let mut sb = TSchema::builder();
    let id_field = sb.add_u64_field("doc_id", INDEXED | STORED);
    let title_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(IndexRecordOption::WithFreqs),
    );
    let title_field = sb.add_text_field("title", title_opts);
    let schema = sb.build();
    let index = Index::create_in_dir(dir.path(), schema.clone()).expect("create_in_dir");
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
    DiskTantivyFixture {
        _dir: dir,
        title_field,
    }
}

fn open_disk_tantivy(path: &Path, title_field: tantivy::schema::Field) -> Index {
    let dir = tantivy::directory::MmapDirectory::open(PathBuf::from(path))
        .expect("mmap directory for disk tantivy index");
    let index = Index::open(dir).expect("open disk tantivy index");
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build();
    index.tokenizers().register("default", analyzer);
    let _ = title_field;
    index
}

fn tantivy_disk_fixture() -> &'static DiskTantivyFixture {
    TANTIVY_DISK.get_or_init(|| {
        eprintln!(
            "[supertable_fts] building disk-backed Tantivy ({N_DOCS} docs) for warm/cold tiers..."
        );
        build_tantivy_on_disk(docs())
    })
}

/// Sequential Tantivy search — used for the df=1 lookup. Cross-segment
/// parallelism for the timed search loop goes through
/// `tantivy_search_parallel` instead.
fn tantivy_search_serial<T>(searcher: &Searcher, q: &dyn Query, collector: &T) -> Vec<(u32, f32)>
where
    T: Collector<Fruit = Vec<(Score, DocAddress)>>,
{
    let top = searcher.search(q, collector).expect("search");
    top.into_iter()
        .map(|(score, addr)| (addr.doc_id, score))
        .collect()
}

/// Tantivy + manual cross-segment parallelism via rayon `par_iter`
/// over `SegmentReader`s. Uses `weight.for_each_pruning` to invoke
/// BooleanWeight's `block_wand` (BMW) — the same skip-pruning class
/// infino uses. Without this, the default `for_each` runs an
/// exhaustive walk that accidentally beats BMW's per-block bookkeeping
/// on rare-term unions, flattering Tantivy in heavy-query parallel
/// timings.
fn tantivy_search_parallel(
    k: usize,
    pool: &ThreadPool,
    searcher: &Searcher,
    weight: Box<dyn Weight>,
) -> Vec<(u32, f32)> {
    use std::cmp::Ordering;
    use std::collections::BinaryHeap;

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

// ─── Query battery ────────────────────────────────────────────────────

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
    eprintln!("[supertable_fts_build] building Tantivy ({N_DOCS} docs)...");
    let _ = tantivy_handles();
    eprintln!("[supertable_fts_build] Tantivy ready");

    let mut g = c.benchmark_group("supertable_fts_build");
    g.sample_size(10);
    g.throughput(Throughput::Elements(N_DOCS as u64));
    let rss_sample = rss::PeakSampler::start_default();

    g.bench_function("tantivy_default_threads", |b| {
        b.iter_with_large_drop(|| build_supertable_tantivy(black_box(docs())));
    });

    g.finish();
    let stats = rss_sample.stop_stats();
    let _ = rss::write_rss_stats(
        group_name::SUPERTABLE_FTS_BUILD,
        "tantivy_default_threads",
        stats,
    );

    emit_ingest_markdown();
    emit_json_results();
}

// ─── Bench: search (group: supertable_fts_search) ─────────────────────

fn bench_search(c: &mut Criterion) {
    let t = tantivy_handles();
    let pool = parallel_pool();
    let qs = battery(t);

    // Quick df=1 sanity using serial Tantivy search.
    let probe_doc_id = (N_DOCS / 2) as u32;
    let probe_token = format!("doc{probe_doc_id:07}");
    let parser = QueryParser::for_index(&t.index, vec![t.title_field]);
    let parsed = parser.parse_query(&probe_token).expect("parse");
    let reader = t.index.reader().expect("reader");
    let searcher = reader.searcher();
    let collector = TopDocs::with_limit(10).order_by_score();
    let hits: Vec<u32> = tantivy_search_serial(&searcher, parsed.as_ref(), &collector)
        .into_iter()
        .map(|(doc, _)| doc)
        .collect();
    assert_eq!(hits.len(), 1, "df=1 sanity: expected one Tantivy hit");
    eprintln!(
        "[supertable_fts_search] df=1 sanity OK (rayon pool: {} threads)",
        pool.current_num_threads()
    );

    let mut g = c.benchmark_group("supertable_fts_hot_search");
    g.sample_size(10);
    let rss_sample = rss::PeakSampler::start_default();

    macro_rules! tantivy_query {
        ($name:literal, $q:expr) => {
            g.bench_function(concat!($name, "_tantivy_top10"), |b| {
                let reader = t.index.reader().expect("reader");
                let searcher = reader.searcher();
                b.iter(|| {
                    let weight = black_box(
                        $q.weight(EnableScoring::enabled_from_searcher(&searcher))
                            .expect("weight"),
                    );
                    let hits = tantivy_search_parallel(TOP_K, &pool, &searcher, weight);
                    black_box(hits)
                });
            });
        };
    }

    tantivy_query!("single_rare", qs.q_single_rare.as_ref());
    tantivy_query!("single_common", qs.q_single_common.as_ref());
    tantivy_query!("two_term_or", qs.q_two.as_ref());
    // OR labels carry the `_or` suffix to match infino's supertable
    // bench output, which retrievalbench reads from disk.
    tantivy_query!("three_wide_or", qs.q_three_wide.as_ref());
    tantivy_query!("three_similar_or", qs.q_three_similar.as_ref());
    tantivy_query!("five_term_or", qs.q_five.as_ref());

    g.bench_function("prefix_tantivy_top10", |b| {
        let reader = t.index.reader().expect("reader");
        let searcher = reader.searcher();
        b.iter(|| {
            let weight = black_box(
                qs.q_prefix
                    .weight(EnableScoring::enabled_from_searcher(&searcher))
                    .expect("weight"),
            );
            let hits = tantivy_search_parallel(TOP_K, &pool, &searcher, weight);
            black_box(hits)
        });
    });

    g.finish();
    let stats = rss_sample.stop_stats();
    for q in QUERY_NAMES {
        let _ = rss::write_rss_stats(
            group_name::SUPERTABLE_FTS_SEARCH,
            &format!("{q}_tantivy_top10"),
            stats,
        );
    }

    bench_search_tantivy_disk_tiers(c, &qs, &pool);

    emit_search_markdown();
    emit_json_results();
}

fn bench_search_tantivy_disk_tiers(c: &mut Criterion, qs: &Battery, pool: &Arc<ThreadPool>) {
    let disk = tantivy_disk_fixture();
    let path = disk._dir.path().to_path_buf();
    let title_field = disk.title_field;

    let queries: &[(&str, &dyn Query)] = &[
        ("single_rare", qs.q_single_rare.as_ref()),
        ("single_common", qs.q_single_common.as_ref()),
        ("two_term_or", qs.q_two.as_ref()),
        ("three_wide_or", qs.q_three_wide.as_ref()),
        ("three_similar_or", qs.q_three_similar.as_ref()),
        ("five_term_or", qs.q_five.as_ref()),
    ];

    for tier in [Tier::Warm, Tier::Cold] {
        let mut g = c.benchmark_group(format!("supertable_fts_{}_search_tantivy", tier.label()));
        g.sample_size(10);

        for (name, query) in queries {
            let bench_id = format!("{name}_supertable_top10");
            match tier {
                Tier::Warm => {
                    let index = open_disk_tantivy(&path, title_field);
                    let reader = index.reader().expect("reader");
                    let searcher = reader.searcher();
                    let weight = query
                        .weight(EnableScoring::enabled_from_searcher(&searcher))
                        .expect("prewarm weight");
                    let _ = tantivy_search_parallel(TOP_K, pool, &searcher, weight);
                    g.bench_function(&bench_id, |b| {
                        let reader = index.reader().expect("reader");
                        let searcher = reader.searcher();
                        b.iter(|| {
                            let weight = query
                                .weight(EnableScoring::enabled_from_searcher(&searcher))
                                .expect("weight");
                            let hits = tantivy_search_parallel(TOP_K, pool, &searcher, weight);
                            black_box(hits)
                        });
                    });
                }
                Tier::Cold => {
                    let path = path.clone();
                    g.bench_function(&bench_id, |b| {
                        b.iter_custom(|iters| {
                            let mut total = Duration::ZERO;
                            for _ in 0..iters {
                                let t0 = Instant::now();
                                let index = open_disk_tantivy(&path, title_field);
                                let reader = index.reader().expect("reader");
                                let searcher = reader.searcher();
                                let weight = query
                                    .weight(EnableScoring::enabled_from_searcher(&searcher))
                                    .expect("weight");
                                let _ = tantivy_search_parallel(TOP_K, pool, &searcher, weight);
                                total += t0.elapsed();
                            }
                            total
                        });
                    });
                }
                Tier::Hot => {}
            }
        }

        g.bench_function("prefix_supertable_top10", |b| match tier {
            Tier::Warm => {
                let index = open_disk_tantivy(&path, title_field);
                let reader = index.reader().expect("reader");
                let searcher = reader.searcher();
                let _ = qs
                    .q_prefix
                    .weight(EnableScoring::enabled_from_searcher(&searcher))
                    .expect("prewarm");
                b.iter(|| {
                    let weight = qs
                        .q_prefix
                        .weight(EnableScoring::enabled_from_searcher(&searcher))
                        .expect("weight");
                    let hits = tantivy_search_parallel(TOP_K, pool, &searcher, weight);
                    black_box(hits)
                });
            }
            Tier::Cold => {
                let path = path.clone();
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        let t0 = Instant::now();
                        let index = open_disk_tantivy(&path, title_field);
                        let reader = index.reader().expect("reader");
                        let searcher = reader.searcher();
                        let weight = qs
                            .q_prefix
                            .weight(EnableScoring::enabled_from_searcher(&searcher))
                            .expect("weight");
                        let _ = tantivy_search_parallel(TOP_K, pool, &searcher, weight);
                        total += t0.elapsed();
                    }
                    total
                });
            }
            Tier::Hot => {}
        });

        g.finish();
    }
}

// ─── JSON results emitter ─────────────────────────────────────────────

fn emit_json_results() {
    let mut collector = results::ResultsCollector::new();

    // Collect build benchmark results
    collector.add_from_criterion(
        group_name::SUPERTABLE_FTS_BUILD,
        "tantivy_default_threads",
        Some("tantivy"),
    );
    collector.add_from_infino(
        group_name::SUPERTABLE_FTS_BUILD,
        "infino_auto_writer_pool",
        Some("infino"),
    );

    // Collect search benchmark results
    for q in QUERY_NAMES {
        collector.add_from_criterion_with_group(
            group_name::SUPERTABLE_FTS_SEARCH,
            &format!("supertable_fts_search_{q}"),
            &format!("{q}_tantivy_top10"),
            Some("tantivy"),
        );
        collector.add_from_infino_with_group(
            group_name::SUPERTABLE_FTS_SEARCH,
            &format!("supertable_fts_search_{q}"),
            &format!("{q}_supertable_top10"),
            Some("infino"),
        );
    }

    if let Err(e) = collector.emit() {
        eprintln!("[results] failed to emit JSON results: {e}");
    }
}

// ─── Markdown summary emitters ────────────────────────────────────────

mod group_name {
    pub const SUPERTABLE_FTS_BUILD: &str = "supertable_fts_build";
    pub const SUPERTABLE_FTS_SEARCH: &str = "supertable_fts_hot_search";
}

const QUERY_NAMES: &[&str] = &[
    "single_rare",
    "single_common",
    "two_term_or",
    "three_wide_or",
    "three_similar_or",
    "five_term_or",
    "prefix",
];

fn emit_ingest_markdown() {
    use markdown::{
        MarkdownSection, fmt_throughput, fmt_time, fmt_winner, read_infino_mean_ns, read_mean_ns,
    };

    let group = group_name::SUPERTABLE_FTS_BUILD;
    let infino_id = "infino_auto_writer_pool";
    let tantivy_id = "tantivy_default_threads";
    let infino_ns = read_infino_mean_ns(group, infino_id);
    let tantivy_ns = read_mean_ns(group, tantivy_id);

    let mut body = String::new();
    body.push_str(&format!(
        "### Supertable FTS — ingest ({N_DOCS} docs, Zipfian, 200 tokens/doc, 10K vocab)\n\n"
    ));
    body.push_str(
        "| Engine | Time | Throughput | Peak RSS | Median RSS | P90 RSS | Peak RSS Δ | vs Tantivy |\n",
    );
    body.push_str(
        "|--------|------|------------|----------|------------|---------|------------|------------|\n",
    );
    for (label, ns, peak_rss, median, p90, delta, is_baseline) in [
        (
            infino_id,
            infino_ns,
            rss::read_infino_peak_rss_bytes(group, infino_id),
            rss::fmt_infino_median_rss(group, infino_id),
            rss::fmt_infino_p90_rss(group, infino_id),
            rss::fmt_infino_peak_rss_delta(group, infino_id),
            false,
        ),
        (
            tantivy_id,
            tantivy_ns,
            rss::read_peak_rss_bytes(group, tantivy_id),
            rss::fmt_median_rss(group, tantivy_id),
            rss::fmt_p90_rss(group, tantivy_id),
            rss::fmt_peak_rss_delta(group, tantivy_id),
            true,
        ),
    ] {
        let time = ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let thrpt = ns
            .map(|n| fmt_throughput((N_DOCS as f64) / (n / 1e9)))
            .unwrap_or_else(|| "—".into());
        let peak = peak_rss.map(rss::fmt_bytes).unwrap_or_else(|| "—".into());
        let cmp = if is_baseline {
            "—".to_string()
        } else {
            fmt_winner("infino", ns, "tantivy", tantivy_ns)
        };
        body.push_str(&format!(
            "| {label} | {time} | {thrpt} | {peak} | {median} | {p90} | {delta} | {cmp} |\n"
        ));
    }
    body.push_str(
        "\n*Output cardinality: infino emits `min(writer_pool.threads, total_rows)` superfiles \
         per commit (auto = cpus/2). Tantivy emits one segment per internal worker thread per \
         commit (≈ 8 × N_chunks segments with NoMergePolicy). Override the infino writer-thread \
         count with `INFINO_SUPERTABLE__WRITER_THREADS=N` to match Tantivy's effective output \
         segment count.*\n",
    );

    markdown::emit(&MarkdownSection {
        anchor_id: "bench/fts/supertable/ingest".into(),
        body,
    });
}

fn emit_search_markdown() {
    use markdown::{MarkdownSection, fmt_time, fmt_winner, read_infino_mean_ns, read_mean_ns};

    let group = group_name::SUPERTABLE_FTS_SEARCH;
    let mut body = String::new();
    body.push_str(&format!("### Supertable FTS — search ({N_DOCS} docs)\n\n"));
    body.push_str(
        "Infino warm/cold from `../infino-pr8` (`supertable_fts_{hot|warm|cold}_search*`). \
         Tantivy warm/cold = disk-backed (`supertable_fts_{warm|cold}_search_tantivy`). \
         Winner = infino hot vs Tantivy hot.\n\n",
    );
    body.push_str(
        "| Query | infino hot | infino warm | infino cold | Tantivy hot | Tantivy warm | Tantivy cold | Winner |\n",
    );
    body.push_str(
        "|-------|------------|-------------|-------------|-------------|--------------|--------------|--------|\n",
    );
    for q in QUERY_NAMES {
        let inf_id = format!("{q}_supertable_top10");
        let tan_hot_id = format!("{q}_tantivy_top10");
        let inf_hot = read_infino_mean_ns(group, &inf_id);
        let inf_warm =
            markdown::read_infino_supertable_tier_mean_ns("supertable_fts", "warm", &inf_id);
        let inf_cold =
            markdown::read_infino_supertable_tier_mean_ns("supertable_fts", "cold", &inf_id);
        let tan_hot = read_mean_ns(group, &tan_hot_id);
        let tan_warm = markdown::read_tantivy_tier_mean_ns("supertable_fts", "warm", &inf_id);
        let tan_cold = markdown::read_tantivy_tier_mean_ns("supertable_fts", "cold", &inf_id);
        let w = fmt_winner("infino", inf_hot, "tantivy", tan_hot);
        body.push_str(&format!(
            "| {q} | {} | {} | {} | {} | {} | {} | {w} |\n",
            inf_hot.map(fmt_time).unwrap_or_else(|| "—".into()),
            inf_warm.map(fmt_time).unwrap_or_else(|| "—".into()),
            inf_cold.map(fmt_time).unwrap_or_else(|| "—".into()),
            tan_hot.map(fmt_time).unwrap_or_else(|| "—".into()),
            tan_warm.map(fmt_time).unwrap_or_else(|| "—".into()),
            tan_cold.map(fmt_time).unwrap_or_else(|| "—".into()),
        ));
    }

    markdown::emit(&MarkdownSection {
        anchor_id: "bench/fts/supertable/search".into(),
        body,
    });
}

criterion_group!(benches, bench_ingest, bench_search);
