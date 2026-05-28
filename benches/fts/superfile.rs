//! Tantivy head-to-head FTS bench for the superfile layer.
//!
//! Measures Tantivy only — infino's own numbers come from
//! `infino/benches` (read out of `../infino/target/criterion/...` at
//! emit time). The corpus, queries, and ground truth are shared
//! between both repos via `infino::test_helpers::bench_corpus`
//! (re-exported via `retrievalbench::corpus`), so the comparison rows in
//! the rendered README are apples-to-apples.
//!
//! Pinned to 1M-doc Zipfian (200 tokens/doc, 10K vocab). The
//! single-superfile shape is rarely much larger in production —
//! `benches/fts/supertable.rs` covers the 10M+ supertable scale.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench fts                            # all FTS
//! cargo bench --bench fts -- superfile_fts_build     # ingest only
//! cargo bench --bench fts -- superfile_fts_search    # search only
//! ```
//!
//! ## Workflow
//!
//! Run infino's bench first so its criterion output is on disk:
//!
//! ```text
//! (cd ../infino && cargo bench --bench fts -- superfile_fts)
//! cargo bench --bench fts -- superfile_fts
//! ```
//!
//! The comparison column shows "—" for infino if infino's criterion
//! output is missing.

use coredb::event_manager::event_reference::EventReference;
use coredb::event_manager::event_reference::FieldReference;
use coredb::event_manager::event_reference::FieldType;
use coredb::index_manager::metadata::static_metadata::StaticMetadata;
use coredb::segment_manager::segment::Segment;
use criterion::{criterion_group, measurement::WallTime, BenchmarkGroup, Criterion, Throughput};
use lancedb::Table;
use retrievalbench::{corpus, lance, markdown, results, rss};
use std::hint::black_box;
use std::sync::OnceLock;
use tantivy::collector::Collector;
use tantivy::collector::TopDocs;
use tantivy::doc;
use tantivy::query::{Query, QueryParser};
use tantivy::schema::{IndexRecordOption, Schema, TextFieldIndexing, TextOptions, INDEXED, STORED};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::DocAddress;
use tantivy::Index;
use tantivy::Score;
use tantivy::Searcher;
use tempfile::TempDir;
use tokio::runtime::Runtime;

// ─── Constants ────────────────────────────────────────────────────────

/// Tantivy heap budget for the indexer at 1M docs.
const TANTIVY_HEAP_BYTES: usize = 500_000_000;

/// CoreDB Segment Config
const COREDB_SEGMENT_EVENT_THRESHOLD: usize = 1_000_000;
const COREDB_NUM_SEGMENTS_IN_MEMORY: usize = 5;
const COREDB_INDEX_NAME: &str = "benchmarkindex";

/// Doc count. Pinned to 1M — the supertable shape is what scales out.
const N_DOCS: usize = 1_000_000;

const TOP_N: usize = 10;

enum TantivyThreads {
    Single,
    Default,
}

struct TantivyHandles {
    index: Index,
    title_field: tantivy::schema::Field,
}

struct CoreDBHandles {
    pub segment: Segment,
}

// Lance data model: a Fragment is one physical `.lance` file (≈ one parquet).
// A Lance Table is a container of N fragments with a single shared index.
// We write all 1M docs in one RecordBatch, so Lance creates one fragment —
// making this bench a single-fragment table. The closest infino analogue is
// a superfile: one shard, one index, 1M docs. Lance's multi-fragment case
// has no direct infino equivalent because Lance uses one shared index over
// all fragments whereas infino's supertable has a separate index per superfile.
struct LanceFtsHandles {
    table: Table,
    _dir: TempDir,
    rt: Runtime,
}

// ─── Fixtures ────────────────────────────────────────────────────────

static DOCS: OnceLock<Vec<String>> = OnceLock::new();
static TANTIVY: OnceLock<TantivyHandles> = OnceLock::new();
static COREDB: OnceLock<CoreDBHandles> = OnceLock::new();
static LANCE_FTS: OnceLock<LanceFtsHandles> = OnceLock::new();

fn docs() -> &'static [String] {
    DOCS.get_or_init(|| corpus::generate_text_corpus(N_DOCS, 1))
        .as_slice()
}

fn tantivy_handles() -> &'static TantivyHandles {
    TANTIVY.get_or_init(|| build_tantivy(docs(), TantivyThreads::Single))
}

fn coredb_handles() -> &'static CoreDBHandles {
    COREDB.get_or_init(|| build_coredb(docs()))
}

fn lance_fts_handles() -> &'static LanceFtsHandles {
    LANCE_FTS.get_or_init(|| {
        // current_thread runtime avoids background worker threads competing
        // for CPU cache during single-query latency measurement — same
        // choice made for the vector benches where it gave 25–53% better p50.
        let rt = Runtime::new().expect("tokio runtime");
        let dir = TempDir::new().expect("tempdir");
        let (table, _) = lance::build_lance_fts_table(&rt, dir.path(), docs(), N_DOCS);
        LanceFtsHandles {
            table,
            _dir: dir,
            rt,
        }
    })
}

// ─── Tantivy builder + search ─────────────────────────────────────────

fn build_tantivy(docs: &[String], threads: TantivyThreads) -> TantivyHandles {
    let mut sb = Schema::builder();
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

    let mut writer = match threads {
        TantivyThreads::Single => index
            .writer_with_num_threads(1, TANTIVY_HEAP_BYTES)
            .expect("writer"),
        TantivyThreads::Default => index.writer(TANTIVY_HEAP_BYTES).expect("writer"),
    };

    for (i, t) in docs.iter().enumerate() {
        writer
            .add_document(doc!(
                id_field => i as u64,
                title_field => t.as_str(),
            ))
            .expect("add_document");
    }
    writer.commit().expect("commit");
    drop(writer);
    TantivyHandles { index, title_field }
}

fn build_coredb(docs: &[String]) -> CoreDBHandles {
    let metadata = StaticMetadata::new(
        COREDB_INDEX_NAME,
        COREDB_SEGMENT_EVENT_THRESHOLD as u32,
        COREDB_SEGMENT_EVENT_THRESHOLD as u32,
        COREDB_NUM_SEGMENTS_IN_MEMORY as u32,
        // Is this tokenizer the same as the one used in tantivy and infino-ai benchmarks?
        "*".to_string(),
        0,
        None,
        0,
        None,
    );
    let segment_id = Segment::create_new_id();
    let tempdir = tempfile::tempdir().expect("should create tempdir");
    let wal_path = tempdir
        .path()
        .join(format!("{}.wal", COREDB_INDEX_NAME))
        .to_string_lossy()
        .to_string();
    let segment_dir_path = tempdir
        .path()
        .join(segment_id.clone())
        .to_string_lossy()
        .to_string();

    let segment = Segment::new(
        &segment_id,
        segment_dir_path.as_str(),
        wal_path.as_str(),
        &metadata.get_segment_type().get_segment_component_flags(),
        COREDB_INDEX_NAME,
        None,
    );

    for (i, doc) in docs.iter().enumerate() {
        let field_reference =
            FieldReference::new_from_string_value("title", doc.to_string(), FieldType::String);
        let message = EventReference::new_with_params(vec![field_reference]);
        segment
            .store_event(i as u32, i as u64, &message, None, "*", None)
            .expect("should append");
    }

    // For term queries, we dont need to commit the segment
    CoreDBHandles { segment: segment }
}

fn tantivy_search_scored<T>(searcher: &Searcher, q: &dyn Query, collector: &T) -> Vec<(u32, f32)>
where
    T: Collector<Fruit = Vec<(Score, DocAddress)>>,
{
    let top = searcher.search(q, collector).expect("search");
    top.into_iter()
        .map(|(score, addr)| (addr.doc_id, score))
        .collect()
}

// ─── Bench helpers ────────────────────────────────────────────────────

fn bench_tantivy_only(
    g: &mut BenchmarkGroup<WallTime>,
    name: &str,
    t: &TantivyHandles,
    tantivy_query: &dyn Query,
) {
    g.bench_function(format!("{name}_tantivy_top10"), |b| {
        let reader = t.index.reader().expect("reader");
        let searcher = reader.searcher();
        let collector = TopDocs::with_limit(TOP_N).order_by_score();
        b.iter(|| {
            let hits = tantivy_search_scored(&searcher, tantivy_query, &collector);
            black_box(hits)
        });
    });
}

fn bench_lance_fts_only(
    g: &mut BenchmarkGroup<WallTime>,
    name: &str,
    lh: &LanceFtsHandles,
    query: &str,
) {
    g.bench_function(format!("{name}_lance_top10"), |b| {
        b.iter(|| {
            let hits = lance::search_lance_fts(&lh.rt, &lh.table, query, TOP_N);
            black_box(hits)
        });
    });
}

fn bench_lance_fts_and_only(
    g: &mut BenchmarkGroup<WallTime>,
    name: &str,
    lh: &LanceFtsHandles,
    terms: &[String],
) {
    // Pre-join outside b.iter() so string allocation is not measured —
    // same pattern as Tantivy where query objects are built before the bench group.
    let query = terms.join(" ");
    g.bench_function(format!("{name}_lance_top10"), |b| {
        b.iter(|| {
            let hits = lance::search_lance_fts_and(&lh.rt, &lh.table, &query, TOP_N);
            black_box(hits)
        });
    });
}

fn bench_coredb_only(
    g: &mut BenchmarkGroup<WallTime>,
    name: &str,
    coredb: &CoreDBHandles,
    terms: Vec<String>,
    term_operator: &str,
    max_size: usize,
) {
    g.bench_function(format!("{name}_coredb_top10"), |b| {
        b.iter(|| {
            let hits = coredb
                .segment
                .search_inverted_index(black_box(terms.clone()), term_operator, max_size)
                .expect("should search");
            black_box(hits)
        });
    });
}

// ─── Bench entry ──────────────────────────────────────────────────────

fn bench(c: &mut Criterion) {
    eprintln!("[fts/superfile] building Tantivy ({N_DOCS} docs)...");
    let t = tantivy_handles();
    eprintln!("[fts/superfile] Tantivy ready");

    let coredb = coredb_handles();
    eprintln!("[fts/superfile] CoreDB ready");

    eprintln!("[fts/superfile] building Lance FTS ({N_DOCS} docs)...");
    let lance_fts = lance_fts_handles();
    eprintln!("[fts/superfile] Lance FTS ready");

    // ---- Ingest sub-bench (group: superfile_fts_build) -------------
    {
        let n = N_DOCS;
        let docs_for_ingest = docs();
        let mut g = c.benchmark_group("superfile_fts_build");
        g.sample_size(10);
        g.throughput(Throughput::Elements(n as u64));
        let rss_sample = rss::PeakSampler::start_default();

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
        g.bench_function(format!("coredb_{n}docs"), |b| {
            b.iter_with_large_drop(|| build_coredb(black_box(docs_for_ingest)));
        });
        g.bench_function(format!("lance_fts_{n}docs"), |b| {
            b.iter_with_large_drop(|| {
                let dir = TempDir::new().expect("tempdir");
                let rt = Runtime::new().expect("tokio runtime");
                let (table, _) =
                    lance::build_lance_fts_table(&rt, dir.path(), black_box(docs_for_ingest), n);
                (table, dir, rt)
            });
        });
        g.finish();
        let stats = rss_sample.stop_stats();
        let _ = rss::write_rss_stats(
            group_name::SUPERFILE_FTS_BUILD,
            &format!("tantivy_1thread_{n}docs"),
            stats,
        );
        let _ = rss::write_rss_stats(
            group_name::SUPERFILE_FTS_BUILD,
            &format!("tantivy_default_threads_{n}docs"),
            stats,
        );
        let _ = rss::write_rss_stats(
            group_name::SUPERFILE_FTS_BUILD,
            &format!("lance_fts_{n}docs"),
            stats,
        );

        emit_ingest_markdown();
    }

    // ---- Search sub-bench (group: superfile_fts_search) ------------
    {
        let q_single_rare_terms = vec!["term09999".to_string()];
        let q_single_df1_terms = vec!["doc0500000".to_string()];
        let q_single_common_terms = vec!["term00001".to_string()];
        let q_two_terms = vec!["term00001".to_string(), "term00050".to_string()];
        let q_three_wide_terms = vec![
            "term00001".to_string(),
            "term00050".to_string(),
            "term00100".to_string(),
        ];
        let q_three_similar_terms = vec![
            "term00050".to_string(),
            "term00051".to_string(),
            "term00052".to_string(),
        ];
        let q_five_terms = vec![
            "term00050".to_string(),
            "term00051".to_string(),
            "term00052".to_string(),
            "term00053".to_string(),
            "term00054".to_string(),
        ];

        let parser = QueryParser::for_index(&t.index, vec![t.title_field]);
        let q_single_rare = parser
            .parse_query(q_single_rare_terms.join(" ").as_str())
            .expect("parse");
        let q_single_df1 = parser
            .parse_query(q_single_df1_terms.join(" ").as_str())
            .expect("parse");
        let q_single_common = parser
            .parse_query(q_single_common_terms.join(" ").as_str())
            .expect("parse");
        let q_two = parser
            .parse_query(q_two_terms.join(" ").as_str())
            .expect("parse");
        let q_three_wide = parser
            .parse_query(q_three_wide_terms.join(" ").as_str())
            .expect("parse");
        let q_three_similar = parser
            .parse_query(q_three_similar_terms.join(" ").as_str())
            .expect("parse");
        let q_five = parser
            .parse_query(q_five_terms.join(" ").as_str())
            .expect("parse");
        let q_two_and = parser
            .parse_query(
                q_two_terms
                    .iter()
                    .map(|t| format!("+{}", t))
                    .collect::<Vec<_>>()
                    .join(" ")
                    .as_str(),
            )
            .expect("parse");
        let q_three_wide_and = parser
            .parse_query(
                q_three_wide_terms
                    .iter()
                    .map(|t| format!("+{}", t))
                    .collect::<Vec<_>>()
                    .join(" ")
                    .as_str(),
            )
            .expect("parse");
        let q_three_similar_and = parser
            .parse_query(
                q_three_similar_terms
                    .iter()
                    .map(|t| format!("+{}", t))
                    .collect::<Vec<_>>()
                    .join(" ")
                    .as_str(),
            )
            .expect("parse");
        let q_five_and = parser
            .parse_query(
                q_five_terms
                    .iter()
                    .map(|t| format!("+{}", t))
                    .collect::<Vec<_>>()
                    .join(" ")
                    .as_str(),
            )
            .expect("parse");

        let mut g = c.benchmark_group("superfile_fts_search");
        let rss_sample = rss::PeakSampler::start_default();

        bench_tantivy_only(&mut g, "single_rare", t, q_single_rare.as_ref());
        bench_coredb_only(
            &mut g,
            "single_rare",
            coredb,
            q_single_rare_terms.clone(),
            "OR",
            TOP_N,
        );
        bench_lance_fts_only(&mut g, "single_rare", lance_fts, &q_single_rare_terms.join(" "));

        bench_tantivy_only(&mut g, "single_df1", t, q_single_df1.as_ref());
        bench_coredb_only(
            &mut g,
            "single_df1",
            coredb,
            q_single_df1_terms.clone(),
            "OR",
            TOP_N,
        );
        bench_lance_fts_only(&mut g, "single_df1", lance_fts, &q_single_df1_terms.join(" "));

        bench_tantivy_only(&mut g, "single_common", t, q_single_common.as_ref());
        bench_coredb_only(
            &mut g,
            "single_common",
            coredb,
            q_single_common_terms.clone(),
            "OR",
            TOP_N,
        );
        bench_lance_fts_only(
            &mut g,
            "single_common",
            lance_fts,
            &q_single_common_terms.join(" "),
        );

        bench_tantivy_only(&mut g, "two_term_or", t, q_two.as_ref());
        bench_coredb_only(
            &mut g,
            "two_term_or",
            coredb,
            q_two_terms.clone(),
            "OR",
            TOP_N,
        );
        bench_lance_fts_only(&mut g, "two_term_or", lance_fts, &q_two_terms.join(" "));

        // OR labels carry the `_or` suffix to match infino's bench
        // labels — retrievalbench reads infino's criterion output by
        // name, and infino writes e.g. `three_wide_or_infino_top10`.
        bench_tantivy_only(&mut g, "three_wide_or", t, q_three_wide.as_ref());
        bench_coredb_only(
            &mut g,
            "three_wide_or",
            coredb,
            q_three_wide_terms.clone(),
            "OR",
            TOP_N,
        );
        bench_lance_fts_only(
            &mut g,
            "three_wide_or",
            lance_fts,
            &q_three_wide_terms.join(" "),
        );

        bench_tantivy_only(&mut g, "three_similar_or", t, q_three_similar.as_ref());
        bench_coredb_only(
            &mut g,
            "three_similar_or",
            coredb,
            q_three_similar_terms.clone(),
            "OR",
            TOP_N,
        );
        bench_lance_fts_only(
            &mut g,
            "three_similar_or",
            lance_fts,
            &q_three_similar_terms.join(" "),
        );

        bench_tantivy_only(&mut g, "five_term_or", t, q_five.as_ref());
        bench_coredb_only(
            &mut g,
            "five_term_or",
            coredb,
            q_five_terms.clone(),
            "OR",
            TOP_N,
        );
        bench_lance_fts_only(&mut g, "five_term_or", lance_fts, &q_five_terms.join(" "));

        bench_tantivy_only(&mut g, "two_term_and", t, q_two_and.as_ref());
        bench_coredb_only(
            &mut g,
            "two_term_and",
            coredb,
            q_two_terms.clone(),
            "AND",
            TOP_N,
        );
        bench_lance_fts_and_only(&mut g, "two_term_and", lance_fts, &q_two_terms);

        bench_tantivy_only(&mut g, "three_wide_and", t, q_three_wide_and.as_ref());
        bench_coredb_only(
            &mut g,
            "three_wide_and",
            coredb,
            q_three_wide_terms.clone(),
            "AND",
            TOP_N,
        );
        bench_lance_fts_and_only(&mut g, "three_wide_and", lance_fts, &q_three_wide_terms);

        bench_tantivy_only(&mut g, "three_similar_and", t, q_three_similar_and.as_ref());
        bench_coredb_only(
            &mut g,
            "three_similar_and",
            coredb,
            q_three_similar_terms.clone(),
            "AND",
            TOP_N,
        );
        bench_lance_fts_and_only(
            &mut g,
            "three_similar_and",
            lance_fts,
            &q_three_similar_terms,
        );

        bench_tantivy_only(&mut g, "five_term_and", t, q_five_and.as_ref());
        bench_coredb_only(
            &mut g,
            "five_term_and",
            coredb,
            q_five_terms.clone(),
            "AND",
            TOP_N,
        );
        bench_lance_fts_and_only(&mut g, "five_term_and", lance_fts, &q_five_terms);

        g.finish();
        let stats = rss_sample.stop_stats();
        for q in QUERY_NAMES_OR.iter().chain(QUERY_NAMES_AND.iter()) {
            let _ = rss::write_rss_stats(
                group_name::SUPERFILE_FTS_SEARCH,
                &format!("{q}_tantivy_top10"),
                stats,
            );
        }
        for q in QUERY_NAMES_OR.iter().chain(QUERY_NAMES_AND.iter()) {
            let _ = rss::write_rss_stats(
                group_name::SUPERFILE_FTS_SEARCH,
                &format!("{q}_lance_top10"),
                stats,
            );
        }

        emit_search_markdown();
    }

    emit_json_results();
}

// ─── JSON results emitter ─────────────────────────────────────────────

fn emit_json_results() {
    let mut collector = results::ResultsCollector::new();

    // Collect build benchmark results
    collector.add_from_criterion(
        group_name::SUPERFILE_FTS_BUILD,
        &format!("tantivy_1thread_{N_DOCS}docs"),
        Some("tantivy_1thread"),
    );
    collector.add_from_criterion(
        group_name::SUPERFILE_FTS_BUILD,
        &format!("tantivy_default_threads_{N_DOCS}docs"),
        Some("tantivy_default_threads"),
    );
    collector.add_from_criterion(
        group_name::SUPERFILE_FTS_BUILD,
        &format!("coredb_{N_DOCS}docs"),
        Some("coredb"),
    );
    collector.add_from_criterion(
        group_name::SUPERFILE_FTS_BUILD,
        &format!("lance_fts_{N_DOCS}docs"),
        Some("lance"),
    );

    // Collect search benchmark results - separate groups per query
    // Note: criterion stores results under "superfile_fts_search", but we organize them by query in results
    for q in QUERY_NAMES_OR {
        let search_group = format!("superfile_fts_search_{q}");
        collector.add_from_criterion_with_group(
            group_name::SUPERFILE_FTS_SEARCH,
            &search_group,
            &format!("{q}_tantivy_top10"),
            Some("tantivy"),
        );
        collector.add_from_criterion_with_group(
            group_name::SUPERFILE_FTS_SEARCH,
            &search_group,
            &format!("{q}_coredb_top10"),
            Some("coredb"),
        );
        collector.add_from_criterion_with_group(
            group_name::SUPERFILE_FTS_SEARCH,
            &search_group,
            &format!("{q}_lance_top10"),
            Some("lance"),
        );
        collector.add_from_infino_with_group(
            group_name::SUPERFILE_FTS_SEARCH,
            &search_group,
            &format!("{q}_infino_top10"),
            Some("infino"),
        );
    }

    for q in QUERY_NAMES_AND {
        let search_group = format!("superfile_fts_search_{q}");
        collector.add_from_criterion_with_group(
            group_name::SUPERFILE_FTS_SEARCH,
            &search_group,
            &format!("{q}_tantivy_top10"),
            Some("tantivy"),
        );
        collector.add_from_criterion_with_group(
            group_name::SUPERFILE_FTS_SEARCH,
            &search_group,
            &format!("{q}_coredb_top10"),
            Some("coredb"),
        );
        collector.add_from_criterion_with_group(
            group_name::SUPERFILE_FTS_SEARCH,
            &search_group,
            &format!("{q}_lance_top10"),
            Some("lance"),
        );
        collector.add_from_infino_with_group(
            group_name::SUPERFILE_FTS_SEARCH,
            &search_group,
            &format!("{q}_infino_top10"),
            Some("infino"),
        );
    }

    // Also collect infino ingest results if available
    collector.add_from_infino(
        group_name::SUPERFILE_FTS_BUILD,
        &format!("infino_1thread_{N_DOCS}docs"),
        Some("infino_1thread"),
    );
    collector.add_from_infino(
        group_name::SUPERFILE_FTS_BUILD,
        &format!("infino_rayon_default_threads_{N_DOCS}docs"),
        Some("infino_rayon_default_threads"),
    );

    if let Err(e) = collector.emit() {
        eprintln!("[results] failed to emit JSON results: {e}");
    }
}

// ─── Markdown summary emitters ────────────────────────────────────────

mod group_name {
    pub const SUPERFILE_FTS_BUILD: &str = "superfile_fts_build";
    pub const SUPERFILE_FTS_SEARCH: &str = "superfile_fts_search";
}

const QUERY_NAMES_OR: &[&str] = &[
    "single_rare",
    "single_df1",
    "single_common",
    "two_term_or",
    "three_wide_or",
    "three_similar_or",
    "five_term_or",
];

const QUERY_NAMES_AND: &[&str] = &[
    "two_term_and",
    "three_wide_and",
    "three_similar_and",
    "five_term_and",
];

fn emit_ingest_markdown() {
    use markdown::{
        fmt_throughput, fmt_time, fmt_winner, read_infino_mean_ns, read_mean_ns, MarkdownSection,
    };

    let mut body = String::new();
    body.push_str(&format!(
        "### Superfile FTS — ingest ({N_DOCS} docs, Zipfian, 200 tokens/doc, 10K vocab)\n\n"
    ));
    body.push_str(
        "| Engine | Time | Throughput | Peak RSS | Median RSS | P90 RSS | Peak RSS Δ | vs Tantivy |\n",
    );
    body.push_str(
        "|--------|------|------------|----------|------------|---------|------------|------------|\n",
    );

    let group = group_name::SUPERFILE_FTS_BUILD;
    let infino_1t_id = format!("infino_1thread_{N_DOCS}docs");
    let infino_rayon_id = format!("infino_rayon_default_threads_{N_DOCS}docs");
    let tantivy_1t_id = format!("tantivy_1thread_{N_DOCS}docs");
    let tantivy_def_id = format!("tantivy_default_threads_{N_DOCS}docs");

    let infino_1t = read_infino_mean_ns(group, &infino_1t_id);
    let infino_rayon = read_infino_mean_ns(group, &infino_rayon_id);
    let tantivy_1t = read_mean_ns(group, &tantivy_1t_id);
    let tantivy_def = read_mean_ns(group, &tantivy_def_id);
    let coredb_ingestion_time = read_mean_ns(group, &format!("coredb_{N_DOCS}docs"));
    let coredb_ingestion_rss = rss::read_peak_rss_bytes(group, &format!("coredb_{N_DOCS}docs"));
    let lance_fts_ingestion_time = read_mean_ns(group, &format!("lance_fts_{N_DOCS}docs"));
    let lance_fts_ingestion_rss =
        rss::read_peak_rss_bytes(group, &format!("lance_fts_{N_DOCS}docs"));

    let infino_row = |label: &str, ns: Option<f64>, bench: &str, baseline: Option<f64>| -> String {
        let time = ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let thrpt = ns
            .map(|n| fmt_throughput((N_DOCS as f64) / (n / 1e9)))
            .unwrap_or_else(|| "—".into());
        let peak = rss::read_infino_peak_rss_bytes(group, bench)
            .map(rss::fmt_bytes)
            .unwrap_or_else(|| "—".into());
        let median = rss::fmt_infino_median_rss(group, bench);
        let p90 = rss::fmt_infino_p90_rss(group, bench);
        let delta = rss::fmt_infino_peak_rss_delta(group, bench);
        let cmp = fmt_winner("infino", ns, "tantivy", baseline);
        format!("| {label} | {time} | {thrpt} | {peak} | {median} | {p90} | {delta} | {cmp} |\n")
    };

    let tantivy_row = |label: &str, ns: Option<f64>, bench: &str| -> String {
        let time = ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let thrpt = ns
            .map(|n| fmt_throughput((N_DOCS as f64) / (n / 1e9)))
            .unwrap_or_else(|| "—".into());
        let peak = rss::read_peak_rss_bytes(group, bench)
            .map(rss::fmt_bytes)
            .unwrap_or_else(|| "—".into());
        let median = rss::fmt_median_rss(group, bench);
        let p90 = rss::fmt_p90_rss(group, bench);
        let delta = rss::fmt_peak_rss_delta(group, bench);
        format!("| {label} | {time} | {thrpt} | {peak} | {median} | {p90} | {delta} | — |\n")
    };

    // Generic row for engines that use write_rss_stats (coredb, lance_fts).
    // Reads peak/median/p90/delta from the same rss store as tantivy_row.
    let row = |label: &str, ns: Option<f64>, _rss: Option<u64>, baseline: Option<f64>, _is_infino: bool| -> String {
        let bench = &format!("{label}_{N_DOCS}docs");
        let time = ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let thrpt = ns
            .map(|n| fmt_throughput((N_DOCS as f64) / (n / 1e9)))
            .unwrap_or_else(|| "—".into());
        let peak = rss::read_peak_rss_bytes(group, bench)
            .map(rss::fmt_bytes)
            .unwrap_or_else(|| "—".into());
        let median = rss::fmt_median_rss(group, bench);
        let p90 = rss::fmt_p90_rss(group, bench);
        let delta = rss::fmt_peak_rss_delta(group, bench);
        let cmp = fmt_winner(label, ns, "tantivy", baseline);
        format!("| {label} | {time} | {thrpt} | {peak} | {median} | {p90} | {delta} | {cmp} |\n")
    };

    body.push_str(&infino_row(
        "infino_1thread",
        infino_1t,
        &infino_1t_id,
        tantivy_1t,
    ));
    body.push_str(&tantivy_row("tantivy_1thread", tantivy_1t, &tantivy_1t_id));
    body.push_str(&infino_row(
        "infino_rayon_default_threads",
        infino_rayon,
        &infino_rayon_id,
        tantivy_def,
    ));
    body.push_str(&tantivy_row(
        "tantivy_default_threads",
        tantivy_def,
        &tantivy_def_id,
    ));
    body.push_str(&row(
        "lance_fts",
        lance_fts_ingestion_time,
        lance_fts_ingestion_rss,
        tantivy_1t,
        false,
    ));

    markdown::emit(&MarkdownSection {
        anchor_id: "bench/fts/superfile/ingest".into(),
        body,
    });
}

fn emit_search_markdown() {
    use markdown::{fmt_time, fmt_winner, read_infino_mean_ns, read_mean_ns, MarkdownSection};

    let mut body = String::new();
    body.push_str(&format!("### Superfile FTS — search ({N_DOCS} docs)\n\n"));

    let group = group_name::SUPERFILE_FTS_SEARCH;
    let row_or = |q: &str, body: &mut String| {
        let inf = read_infino_mean_ns(group, &format!("{q}_infino_top10"));
        let tan = read_mean_ns(group, &format!("{q}_tantivy_top10"));
        let coredb = read_mean_ns(group, &format!("{q}_coredb_top10"));
        let lance = read_mean_ns(group, &format!("{q}_lance_top10"));
        let inf_s = inf.map(fmt_time).unwrap_or_else(|| "—".into());
        let tan_s = tan.map(fmt_time).unwrap_or_else(|| "—".into());
        let coredb_s = coredb.map(fmt_time).unwrap_or_else(|| "—".into());
        let lance_s = lance.map(fmt_time).unwrap_or_else(|| "—".into());
        let inf_rss = rss::read_infino_peak_rss_bytes(group, &format!("{q}_infino_top10"))
            .map(rss::fmt_bytes)
            .unwrap_or_else(|| "—".into());
        let tan_rss = rss::read_peak_rss_bytes(group, &format!("{q}_tantivy_top10"))
            .map(rss::fmt_bytes)
            .unwrap_or_else(|| "—".into());
        let coredb_rss = rss::read_peak_rss_bytes(group, &format!("{q}_coredb_top10"))
            .map(rss::fmt_bytes)
            .unwrap_or_else(|| "—".into());
        let lance_rss = rss::read_peak_rss_bytes(group, &format!("{q}_lance_top10"))
            .map(rss::fmt_bytes)
            .unwrap_or_else(|| "—".into());
        // TODO: Add support for CoreDB winner/comparisons
        let w = fmt_winner("infino", inf, "tantivy", tan);
        body.push_str(&format!(
            "| {q:17} | {inf_s:10} | {inf_rss:10} | {tan_s:10} | {tan_rss:11} | {coredb_s:10} | {coredb_rss:10} | {lance_s:10} | {lance_rss:10} | {w:21} |\n"
        ));
    };
    // Each section emits its own header + separator so the OR / AND
    // groups render as two valid markdown tables rather than one big
    // table with a bold heading injected mid-rows.
    {
        body.push_str("**OR queries:**\n\n");
        body.push_str("| Query             | infino     | infino RSS | Tantivy    | Tantivy RSS | CoreDB    | CoreDB RSS  | Lance FTS  | Lance RSS  | Winner                |\n");
        body.push_str("|-------------------|------------|------------|------------|-------------|------------|------------|------------|------------|-----------------------|\n");
        for q in QUERY_NAMES_OR {
            row_or(q, &mut body);
        }
        body.push('\n');
    }
    {
        body.push_str("**AND queries:**\n\n");
        body.push_str("| Query             | infino     | infino RSS | Tantivy    | Tantivy RSS | CoreDB    | CoreDB RSS  | Lance FTS  | Lance RSS  | Winner                |\n");
        body.push_str("|-------------------|------------|------------|------------|-------------|------------|------------|------------|------------|-----------------------|\n");
        for q in QUERY_NAMES_AND {
            row_or(q, &mut body); // row_or reads Lance col too; same format for AND
        }
        body.push('\n');
    }

    body.push('\n');
    body.push_str("**Per-algorithm probes** (infino-only, WAND+BMW vs MaxScore+BMM):\n\n");
    body.push_str(
        "| Shape | WAND+BMW p50 | WAND+BMW Peak RSS | WAND+BMW Median RSS | WAND+BMW P90 RSS | WAND+BMW Peak RSS Δ | MaxScore+BMM p50 | MaxScore+BMM Peak RSS | MaxScore+BMM Median RSS | MaxScore+BMM P90 RSS | MaxScore+BMM Peak RSS Δ | Winner |\n",
    );
    body.push_str(
        "|-------|--------------|-------------------|---------------------|------------------|---------------------|------------------|-----------------------|-------------------------|----------------------|-------------------------|--------|\n",
    );
    // Per-algo probe labels carry the `_or` suffix in infino's bench
    // output (e.g. `wide_3_or_wand_top10`) — the lookup key matches.
    for shape in ["wide_3_or", "similar_3_or", "similar_5_or"] {
        let wand_id = format!("{shape}_wand_top10");
        let bmm_id = format!("{shape}_bmm_top10");
        let wand = read_infino_mean_ns(group, &wand_id);
        let bmm = read_infino_mean_ns(group, &bmm_id);
        let wand_s = wand.map(fmt_time).unwrap_or_else(|| "—".into());
        let bmm_s = bmm.map(fmt_time).unwrap_or_else(|| "—".into());
        let wand_peak = rss::read_infino_peak_rss_bytes(group, &wand_id)
            .map(rss::fmt_bytes)
            .unwrap_or_else(|| "—".into());
        let wand_median = rss::fmt_infino_median_rss(group, &wand_id);
        let wand_p90 = rss::fmt_infino_p90_rss(group, &wand_id);
        let wand_delta = rss::fmt_infino_peak_rss_delta(group, &wand_id);
        let bmm_peak = rss::read_infino_peak_rss_bytes(group, &bmm_id)
            .map(rss::fmt_bytes)
            .unwrap_or_else(|| "—".into());
        let bmm_median = rss::fmt_infino_median_rss(group, &bmm_id);
        let bmm_p90 = rss::fmt_infino_p90_rss(group, &bmm_id);
        let bmm_delta = rss::fmt_infino_peak_rss_delta(group, &bmm_id);
        let w = fmt_winner("WAND+BMW", wand, "MaxScore+BMM", bmm);
        body.push_str(&format!(
            "| {shape} | {wand_s} | {wand_peak} | {wand_median} | {wand_p90} | {wand_delta} | {bmm_s} | {bmm_peak} | {bmm_median} | {bmm_p90} | {bmm_delta} | {w} |\n"
        ));
    }

    markdown::emit(&MarkdownSection {
        anchor_id: "bench/fts/superfile/search".into(),
        body,
    });
}

criterion_group!(benches, bench);
