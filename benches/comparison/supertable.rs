// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Supertable object-store comparison bench.
//!
//! Mirrors the ingest shape of infino's own supertable bench (`cargo bench
//! -- supertable`) and uses the existing Infino supertable bench utilities
//! as the baseline source of truth.

use std::sync::Arc;
use std::time::{Duration, Instant};

use infino_bench_utils::corpus::{self, MmapTextCorpus, MmapVectorCorpus};
use infino_bench_utils::harness::{
    BoolMode, FtsQuery, SqlQuery, SqlRunConfig, VectorMetric, VectorQuery, VectorRunConfig,
    VectorSearch, run_fts, run_fts_with_index, run_sql, run_sql_with_index, run_vector,
    run_vector_with_index,
};
use infino_bench_utils::ingest::supertable::{self, TEXT_COLUMN, VEC_COLUMN};
use infino_bench_utils::markdown::{fmt_count, fmt_throughput, fmt_time};
use infino_bench_utils::report::{Better, Block, Cell, Report, Section, metric, text};
use infino_bench_utils::rss;
use infino_bench_utils::superfile::sql::sql_rows;
use infino_bench_utils::supertable::{
    handle_shape_child_from_env, ingest_row, run_ingest_shapes_isolated,
};
use infino_bench_utils::tiers;
use retrievalbench::{LanceS3FtsEngine, LanceS3SqlEngine, LanceS3VectorEngine};

const EMPTY_FTS_QUERIES: &[FtsQuery] = &[];
const EMPTY_VECTOR_QUERIES: &[VectorQuery<'_>] = &[];
const EMPTY_SQL_QUERIES: &[SqlQuery] = &[];
const WARM_ITERS: usize = 20;
const COLD_ITERS: usize = 5;
const TOP_K: usize = 10;

fn p50(samples: &mut [Duration]) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    samples.sort_unstable();
    samples[(samples.len() - 1) / 2]
}

fn pct(peer: f64, baseline: f64) -> String {
    if baseline == 0.0 {
        return "N/A".into();
    }
    let p = (peer - baseline) / baseline * 100.0;
    if p > 0.0 {
        format!("+{p:.1}%")
    } else {
        format!("{p:.1}%")
    }
}

fn emit_latency_comparison(
    report: &mut Report,
    anchor: &str,
    title: String,
    note: &str,
    label: &str,
    infino: &[(&'static str, Duration)],
    lance: &[(&'static str, Duration)],
) {
    let mut rows = Vec::new();
    for (name, infino_d) in infino {
        let Some((_, lance_d)) = lance.iter().find(|(n, _)| n == name) else {
            continue;
        };
        let infino_ns = infino_d.as_secs_f64() * 1e9;
        let lance_ns = lance_d.as_secs_f64() * 1e9;
        rows.push(vec![
            text(*name),
            metric(infino_ns, fmt_time(infino_ns), Better::Lower),
            metric(lance_ns, fmt_time(lance_ns), Better::Lower),
            metric(
                lance_ns - infino_ns,
                pct(lance_ns, infino_ns),
                Better::Lower,
            ),
        ]);
    }
    report.emit(&Section {
        anchor: anchor.into(),
        title,
        note: note.into(),
        blocks: vec![Block {
            subtitle: label.into(),
            headers: vec![
                "Query".into(),
                "infino".into(),
                "lancedb-s3".into(),
                "lancedb Δ".into(),
            ],
            rows,
        }],
    });
}

fn lance_fts_ingest_row(n_docs: usize) -> Vec<Cell> {
    eprintln!(
        "[comparison-supertable] building LanceDB FTS-only peer on S3 over {} docs...",
        fmt_count(n_docs)
    );
    let corpus = MmapTextCorpus::generate(n_docs, 1);
    let docs = corpus.rows();
    let result = run_fts::<LanceS3FtsEngine>(TEXT_COLUMN, &docs, EMPTY_FTS_QUERIES, 10, 1, 1);
    let build = result.builds.first().expect("lancedb-s3 build row");
    let secs = build.phase.wall.as_secs_f64();
    let wall_ns = secs * 1e9;
    let throughput = if secs > 0.0 {
        n_docs as f64 / secs
    } else {
        0.0
    };
    vec![
        text("LanceDB FTS-only"),
        metric(wall_ns, fmt_time(wall_ns), Better::Lower),
        metric(throughput, fmt_throughput(throughput), Better::Higher),
        text("—"),
        metric(
            build.phase.rss.peak_rss_bytes as f64,
            rss::fmt_bytes(build.phase.rss.peak_rss_bytes),
            Better::Lower,
        ),
        metric(
            build.phase.rss.median_rss_bytes as f64,
            rss::fmt_bytes(build.phase.rss.median_rss_bytes),
            Better::Lower,
        ),
        metric(
            build.phase.rss.p90_rss_bytes as f64,
            rss::fmt_bytes(build.phase.rss.p90_rss_bytes),
            Better::Lower,
        ),
    ]
}

fn lance_vector_ingest_row(n_docs: usize) -> Vec<Cell> {
    eprintln!(
        "[comparison-supertable] building LanceDB vector-only peer on S3 over {} docs...",
        fmt_count(n_docs)
    );
    let vectors = MmapVectorCorpus::generate(n_docs, corpus::n_cent(n_docs), 1, true);
    let cfg = VectorRunConfig {
        column: VEC_COLUMN,
        dim: corpus::DIM,
        metric: VectorMetric::Cosine,
        k: 10,
        iters: 1,
        parallel: 1,
    };
    let result = run_vector::<LanceS3VectorEngine>(cfg, vectors.as_slice(), EMPTY_VECTOR_QUERIES);
    let build = result.builds.first().expect("lancedb-s3 vector build row");
    let secs = build.wall.as_secs_f64();
    let wall_ns = secs * 1e9;
    let throughput = if secs > 0.0 {
        n_docs as f64 / secs
    } else {
        0.0
    };
    vec![
        text("LanceDB vector-only"),
        metric(wall_ns, fmt_time(wall_ns), Better::Lower),
        metric(throughput, fmt_throughput(throughput), Better::Higher),
        text("—"),
        metric(
            build.rss.peak_rss_bytes as f64,
            rss::fmt_bytes(build.rss.peak_rss_bytes),
            Better::Lower,
        ),
        metric(
            build.rss.median_rss_bytes as f64,
            rss::fmt_bytes(build.rss.median_rss_bytes),
            Better::Lower,
        ),
        metric(
            build.rss.p90_rss_bytes as f64,
            rss::fmt_bytes(build.rss.p90_rss_bytes),
            Better::Lower,
        ),
    ]
}

fn lance_sql_ingest_row(n_docs: usize) -> Vec<Cell> {
    eprintln!(
        "[comparison-supertable] building LanceDB SQL peer on S3 over {} docs...",
        fmt_count(n_docs)
    );
    let corpus = MmapTextCorpus::generate(n_docs, 1);
    let corpus_rows = corpus.rows();
    let rows = sql_rows(&corpus_rows);
    let cfg = SqlRunConfig {
        iters: 1,
        parallel: 1,
    };
    let result = run_sql::<LanceS3SqlEngine>(cfg, &rows, EMPTY_SQL_QUERIES);
    let build = result.builds.first().expect("lancedb-s3 sql build row");
    let secs = build.wall.as_secs_f64();
    let wall_ns = secs * 1e9;
    let throughput = if secs > 0.0 {
        n_docs as f64 / secs
    } else {
        0.0
    };
    vec![
        text("LanceDB SQL"),
        metric(wall_ns, fmt_time(wall_ns), Better::Lower),
        metric(throughput, fmt_throughput(throughput), Better::Higher),
        text("—"),
        metric(
            build.rss.peak_rss_bytes as f64,
            rss::fmt_bytes(build.rss.peak_rss_bytes),
            Better::Lower,
        ),
        metric(
            build.rss.median_rss_bytes as f64,
            rss::fmt_bytes(build.rss.median_rss_bytes),
            Better::Lower,
        ),
        metric(
            build.rss.p90_rss_bytes as f64,
            rss::fmt_bytes(build.rss.p90_rss_bytes),
            Better::Lower,
        ),
    ]
}

pub fn run() {
    // Each supertable cell (fts / vector / sql) requests the ingest
    // comparison during its build phase; the measurement is identical, so
    // emit it once per process instead of once per cell.
    static INGEST_ONCE: std::sync::Once = std::sync::Once::new();
    let mut first = false;
    INGEST_ONCE.call_once(|| first = true);
    if !first {
        return;
    }

    if let Err(reason) = tiers::supertable_backend_check() {
        eprintln!("[comparison-supertable] skipped: {reason}");
        return;
    }

    if handle_shape_child_from_env() {
        return;
    }

    let n_docs = supertable::n_docs();
    eprintln!(
        "[comparison-supertable] ingesting {} docs ({} commits) per Infino supertable shape...",
        fmt_count(n_docs),
        supertable::n_commits()
    );

    let shape_results = run_ingest_shapes_isolated();
    let mut rows = shape_results
        .iter()
        .map(|r| ingest_row(n_docs, r.label, &r.metrics))
        .collect::<Vec<_>>();
    rows.push(lance_fts_ingest_row(n_docs));
    rows.push(lance_vector_ingest_row(n_docs));
    rows.push(lance_sql_ingest_row(n_docs));

    if rows.is_empty() {
        eprintln!("[comparison-supertable] no Infino baseline rows produced — not emitting report");
        return;
    }

    let mut report = Report::load_plain("comparison-supertable");
    report.emit(&Section {
        anchor: "comparison/supertable/ingest".into(),
        title: format!(
            "Supertable comparison — ingest, multi-segment / object-store ({} docs × dim={}, {} commits)",
            fmt_count(n_docs),
            infino_bench_utils::corpus::DIM,
            supertable::n_commits()
        ),
        note: "Infino baseline rows are produced by the same isolated shape measurement as infino's own supertable bench: `SupertableWriter::append` + `commit` to object storage, one subprocess per shape. Peer rows use existing comparison drivers with public object-store configuration; LanceDB FTS/vector/SQL rows are driven by `run_fts`/`run_vector`/`run_sql` with S3-configured adapters.".into(),
        blocks: vec![Block {
            subtitle: "Ingest".into(),
            headers: vec![
                "Shape".into(),
                "Time".into(),
                "Throughput".into(),
                "Superfiles".into(),
                "Peak RSS".into(),
                "Median RSS".into(),
                "P90 RSS".into(),
            ],
            rows,
        }],
    });
    report.save();
}

#[allow(dead_code)]
fn main() {
    run();
}

pub mod fts {
    use super::*;
    use infino::superfile::fts::reader::BoolMode as InfinoBoolMode;

    pub fn run(build: bool, warm: bool, cold: bool) {
        if let Err(reason) = tiers::supertable_backend_check() {
            eprintln!("[comparison-supertable-fts] skipped: {reason}");
            return;
        }
        if build {
            // Preserve the existing ingest comparison section for the build
            // phase until the build tables are split by selector.
            super::run();
        }

        if !(warm || cold) {
            return;
        }

        let n_docs = supertable::n_docs();
        let infino_corpus = supertable::prepare_corpus(supertable::Modality::Fts);
        let infino_built = supertable::build_on_storage(supertable::Modality::Fts, &infino_corpus);
        let corpus = MmapTextCorpus::generate(n_docs, 1);
        let docs = corpus.rows();
        let (lance_warm, lance_index) = run_fts_with_index::<LanceS3FtsEngine>(
            TEXT_COLUMN,
            &docs,
            infino_bench_utils::executors::fts::FTS_BATTERY,
            TOP_K,
            WARM_ITERS,
            1,
        );

        let mut report = Report::load_plain("comparison-supertable-fts");
        if warm {
            let infino_warm = measure_infino_warm(&infino_built);
            let lance_warm_rows: Vec<_> =
                lance_warm.queries.iter().map(|q| (q.name, q.p50)).collect();
            emit_latency_comparison(
                &mut report,
                "comparison/supertable/fts/warm",
                format!(
                    "Supertable FTS comparison — warm search ({} docs)",
                    fmt_count(n_docs)
                ),
                "warm = object-store table opened with a warm consumer/cache. Engines without this tier are omitted.",
                "warm",
                &infino_warm,
                &lance_warm_rows,
            );
        }
        if cold {
            let infino_cold = measure_infino_cold(&infino_built);
            let lance_cold = measure_lance_cold(&lance_index);
            emit_latency_comparison(
                &mut report,
                "comparison/supertable/fts/cold",
                format!(
                    "Supertable FTS comparison — cold search ({} docs)",
                    fmt_count(n_docs)
                ),
                "Cold = fresh object-store read path per iteration; rebuild time is excluded.",
                "cold",
                &infino_cold,
                &lance_cold,
            );
        }
        report.save();

        if let Some(cleanup) = &infino_built.cleanup {
            tiers::cleanup_prefix(cleanup);
        }
    }

    fn to_infino_mode(mode: BoolMode) -> InfinoBoolMode {
        match mode {
            BoolMode::Or => InfinoBoolMode::Or,
            BoolMode::And => InfinoBoolMode::And,
        }
    }

    fn open_infino_consumer(
        built: &supertable::IngestResult,
    ) -> (tempfile::TempDir, infino::supertable::Supertable) {
        let (cache_dir, cache) = tiers::fresh_supertable_search_cache(
            Arc::clone(&built.storage),
            Some(built.total_index_bytes),
        );
        let opts = tiers::consumer_options(
            supertable::options_for(supertable::Modality::Fts, None),
            Arc::clone(&built.storage),
            cache,
        );
        (cache_dir, tiers::open_consumer(opts))
    }

    fn measure_infino_warm(built: &supertable::IngestResult) -> Vec<(&'static str, Duration)> {
        let (_cache_dir, table) = open_infino_consumer(built);
        infino_bench_utils::executors::fts::FTS_BATTERY
            .iter()
            .map(|q| {
                let query = q.terms.join(" ");
                let mode = to_infino_mode(q.mode);
                let reader = table.reader();
                let _ = reader
                    .bm25_search(TEXT_COLUMN, &query, TOP_K, mode, None)
                    .expect("warmup infino bm25");
                let mut samples = Vec::with_capacity(WARM_ITERS);
                for _ in 0..WARM_ITERS {
                    let t = Instant::now();
                    let hits = reader
                        .bm25_search(TEXT_COLUMN, &query, TOP_K, mode, None)
                        .expect("warm infino bm25");
                    std::hint::black_box(hits);
                    samples.push(t.elapsed());
                }
                (q.name, p50(&mut samples))
            })
            .collect()
    }

    fn measure_infino_cold(built: &supertable::IngestResult) -> Vec<(&'static str, Duration)> {
        infino_bench_utils::executors::fts::FTS_BATTERY
            .iter()
            .map(|q| {
                let query = q.terms.join(" ");
                let mode = to_infino_mode(q.mode);
                let mut samples = Vec::with_capacity(COLD_ITERS);
                for _ in 0..COLD_ITERS {
                    let (cache_dir, table) = open_infino_consumer(built);
                    let t = Instant::now();
                    let hits = table
                        .reader()
                        .bm25_search(TEXT_COLUMN, &query, TOP_K, mode, None)
                        .expect("cold infino bm25");
                    std::hint::black_box(hits);
                    samples.push(t.elapsed());
                    drop(table);
                    drop(cache_dir);
                }
                (q.name, p50(&mut samples))
            })
            .collect()
    }

    fn measure_lance_cold(
        index: &retrievalbench::lance::fts::LanceFtsIndex,
    ) -> Vec<(&'static str, Duration)> {
        infino_bench_utils::executors::fts::FTS_BATTERY
            .iter()
            .map(|q| {
                let mut samples = Vec::with_capacity(COLD_ITERS);
                for _ in 0..COLD_ITERS {
                    let table = index.cold_open();
                    let t = Instant::now();
                    let hits = index.cold_search(&table, q.terms, TOP_K, q.mode);
                    std::hint::black_box(hits);
                    samples.push(t.elapsed());
                    drop(table);
                }
                (q.name, p50(&mut samples))
            })
            .collect()
    }
}

pub mod vector {
    use super::*;
    use crate::superfile::vector::calibrated_rows;
    use infino::superfile::reader::VectorSearchOptions;
    use infino_bench_utils::executors::vector as exec_vec;

    pub fn run(build: bool, warm: bool, cold: bool) {
        if let Err(reason) = tiers::supertable_backend_check() {
            eprintln!("[comparison-supertable-vector] skipped: {reason}");
            return;
        }
        if build {
            super::run();
        }
        if !(warm || cold) {
            return;
        }

        let n_docs = supertable::n_docs();
        let infino_corpus = supertable::prepare_corpus(supertable::Modality::Vector);
        let infino_built =
            supertable::build_on_storage(supertable::Modality::Vector, &infino_corpus);
        // Same seed as the ingested corpus (`CORPUS_VEC_SEED = 1`), so the
        // regenerated vectors are bit-identical to what was committed and
        // brute-force ground truth is valid for both engines.
        let vectors = MmapVectorCorpus::generate(n_docs, corpus::n_cent(n_docs), 1, true);
        let cfg = VectorRunConfig {
            column: VEC_COLUMN,
            dim: corpus::DIM,
            metric: VectorMetric::Cosine,
            k: TOP_K,
            iters: WARM_ITERS,
            parallel: 1,
        };
        // Build only; warm rows come from the recall-calibrated protocol.
        let (_lance_build, lance_index) = run_vector_with_index::<LanceS3VectorEngine>(
            cfg,
            vectors.as_slice(),
            EMPTY_VECTOR_QUERIES,
        );

        let mut report = Report::load_plain("comparison-supertable-vector");
        {
            // Shared fixtures (same seeds/protocol as infino's own
            // supertable vector bench). Calibration runs for either phase:
            // warm rows time the calibrated points on warm readers; cold
            // rows re-time the SAME points on fresh consumers/tables —
            // higher recall targets probe more clusters, which on the cold
            // path means more object-store fetches, so cold must walk the
            // recall axis too.
            let vslice = vectors.as_slice();
            let q_corr = corpus::generate_realistic_queries(
                vslice,
                n_docs,
                exec_vec::N_CORRECTNESS_QUERIES,
                exec_vec::QUERY_CORRECTNESS_SEED,
                true,
                exec_vec::QUERY_SIGMA,
            );
            let gt_corr = corpus::ground_truth(vslice, n_docs, &q_corr, TOP_K);
            let q_cal = corpus::generate_realistic_queries(
                vslice,
                n_docs,
                exec_vec::N_CALIBRATION_QUERIES,
                exec_vec::QUERY_CALIBRATION_SEED,
                true,
                exec_vec::QUERY_SIGMA,
            );
            let gt_cal = corpus::ground_truth(vslice, n_docs, &q_cal, TOP_K);

            // Infino: warm consumer, promoted to the mmap tier like the
            // in-tree supertable bench does before its warm rows.
            let (_cache_dir, table) = open_infino_consumer(&infino_built);
            let _ = table
                .reader()
                .vector_search(VEC_COLUMN, &q_cal[0], TOP_K, search_opts(), None)
                .expect("warm prewarm vector_search");
            table
                .wait_until_warm(Duration::from_secs(600))
                .expect("supertable warm promotion");

            // Correctness gate doubles as a cache warmer for both engines
            // (one full pass over the correctness battery each).
            let infino_gate = exec_vec::mean_recall(
                &table,
                VEC_COLUMN,
                &q_corr,
                &gt_corr,
                TOP_K,
                exec_vec::CORRECTNESS_NPROBE,
                exec_vec::CORRECTNESS_RERANK_MULT,
            );
            let lance_gate = exec_vec::mean_recall(
                &lance_index,
                VEC_COLUMN,
                &q_corr,
                &gt_corr,
                TOP_K,
                exec_vec::CORRECTNESS_NPROBE,
                exec_vec::CORRECTNESS_RERANK_MULT,
            );
            eprintln!(
                "[comparison-supertable-vector] correctness recall@{TOP_K}: infino={infino_gate:.3} lancedb-s3={lance_gate:.3}",
            );

            let infino_rows = calibrated_rows(&table, "infino", VEC_COLUMN, &q_cal, &gt_cal);
            let lance_rows =
                calibrated_rows(&lance_index, "lancedb-s3", VEC_COLUMN, &q_cal, &gt_cal);

            const NS_PER_SEC: f64 = 1e9;
            let mut rows = Vec::new();
            for (inf, lan) in infino_rows.iter().zip(&lance_rows) {
                let fmt_point = |p: Option<(usize, usize)>| match p {
                    Some((probe, refine)) => format!("p={probe}, r={refine}"),
                    None => "—".into(),
                };
                let fmt_recall = |r: f32| {
                    if r.is_nan() {
                        "—".into()
                    } else {
                        format!("{r:.3}")
                    }
                };
                let mut row = vec![
                    text(inf.label.clone()),
                    text(fmt_point(inf.point)),
                    text(fmt_recall(inf.recall)),
                ];
                if warm {
                    row.push(metric(inf.p50_ns, fmt_time(inf.p50_ns), Better::Lower));
                }
                if cold {
                    match inf.point {
                        Some((probe, refine)) => {
                            let c = cold_p50_infino(&infino_built, &q_cal[0], probe, refine);
                            row.push(metric(
                                c.as_secs_f64() * NS_PER_SEC,
                                fmt_time(c.as_secs_f64() * NS_PER_SEC),
                                Better::Lower,
                            ));
                        }
                        None => row.push(text(String::from("—"))),
                    }
                }
                row.push(text(fmt_point(lan.point)));
                row.push(text(fmt_recall(lan.recall)));
                match lan.point {
                    Some((probe, refine)) => {
                        if warm {
                            row.push(metric(lan.p50_ns, fmt_time(lan.p50_ns), Better::Lower));
                        }
                        if cold {
                            let c = cold_p50_lance(&lance_index, &q_cal[0], probe, refine);
                            row.push(metric(
                                c.as_secs_f64() * NS_PER_SEC,
                                fmt_time(c.as_secs_f64() * NS_PER_SEC),
                                Better::Lower,
                            ));
                        }
                        if warm {
                            row.push(metric(
                                lan.p50_ns - inf.p50_ns,
                                pct(lan.p50_ns, inf.p50_ns),
                                Better::Lower,
                            ));
                        }
                    }
                    None => {
                        let blanks = usize::from(warm) * 2 + usize::from(cold);
                        for _ in 0..blanks {
                            row.push(text(String::from("—")));
                        }
                    }
                }
                rows.push(row);
            }

            let mut headers = vec![
                "Recall target".into(),
                "infino (p, r)".into(),
                "infino recall".into(),
            ];
            if warm {
                headers.push("infino warm".into());
            }
            if cold {
                headers.push("infino cold".into());
            }
            headers.push("lancedb-s3 (p, r)".into());
            headers.push("lancedb-s3 recall".into());
            if warm {
                headers.push("lancedb-s3 warm".into());
            }
            if cold {
                headers.push("lancedb-s3 cold".into());
            }
            if warm {
                headers.push("lancedb-s3 warm Δ".into());
            }

            report.emit(&Section {
                anchor: "comparison/supertable/vector".into(),
                title: format!(
                    "Supertable vector comparison — recall-calibrated search ({} docs × dim={})",
                    fmt_count(n_docs),
                    corpus::DIM
                ),
                note: format!(
                    "Same corpus, queries, ground truth, and calibration grid as infino's \
                     own supertable vector bench; rows compare latency at matched recall. \
                     warm = promoted consumer cache / warmed table; cold = fresh consumer \
                     or fresh table per iteration at the SAME calibrated point (higher \
                     recall probes more clusters, so cold cost rises with the target). \
                     Correctness gate recall@{TOP_K}: infino {infino_gate:.3}, lancedb-s3 \
                     {lance_gate:.3}. Engines without this tier are omitted."
                ),
                blocks: vec![Block {
                    subtitle: "Search — recall-calibrated".into(),
                    headers,
                    rows,
                }],
            });
        }
        report.save();

        if let Some(cleanup) = &infino_built.cleanup {
            tiers::cleanup_prefix(cleanup);
        }
    }

    fn open_infino_consumer(
        built: &supertable::IngestResult,
    ) -> (tempfile::TempDir, infino::supertable::Supertable) {
        let (cache_dir, cache) = tiers::fresh_supertable_search_cache(
            Arc::clone(&built.storage),
            Some(built.total_index_bytes),
        );
        let opts = tiers::consumer_options(
            supertable::options_for(supertable::Modality::Vector, None),
            Arc::clone(&built.storage),
            cache,
        );
        (cache_dir, tiers::open_consumer(opts))
    }

    fn search_opts() -> VectorSearchOptions {
        VectorSearchOptions::new()
            .with_nprobe(8)
            .with_rerank_mult(20)
    }

    /// Cold p50 at one calibrated `(probe, refine)` point: a fresh
    /// consumer + cache per iteration, timing the first search.
    fn cold_p50_infino(
        built: &supertable::IngestResult,
        query: &[f32],
        probe: usize,
        refine: usize,
    ) -> Duration {
        let opts = VectorSearchOptions::new()
            .with_nprobe(probe)
            .with_rerank_mult(refine);
        let mut samples = Vec::with_capacity(COLD_ITERS);
        for _ in 0..COLD_ITERS {
            let (cache_dir, table) = open_infino_consumer(built);
            let t = Instant::now();
            let hits = table
                .reader()
                .vector_search(VEC_COLUMN, query, TOP_K, opts, None)
                .expect("cold infino vector");
            std::hint::black_box(hits);
            samples.push(t.elapsed());
            drop(table);
            drop(cache_dir);
        }
        p50(&mut samples)
    }

    /// Cold p50 at one calibrated `(probe, refine)` point: a fresh table
    /// open from object storage per iteration.
    fn cold_p50_lance(
        index: &retrievalbench::lance::vector::LanceVectorIndex,
        query: &[f32],
        probe: usize,
        refine: usize,
    ) -> Duration {
        let search = VectorSearch {
            nprobe: probe,
            rerank_mult: refine,
        };
        let mut samples = Vec::with_capacity(COLD_ITERS);
        for _ in 0..COLD_ITERS {
            let table = index.cold_open();
            let t = Instant::now();
            let hits = index.cold_search(&table, query, TOP_K, search);
            std::hint::black_box(hits);
            samples.push(t.elapsed());
            drop(table);
        }
        p50(&mut samples)
    }
}

pub mod sql {
    use super::*;

    pub fn run(build: bool, warm: bool, cold: bool) {
        if let Err(reason) = tiers::supertable_backend_check() {
            eprintln!("[comparison-supertable-sql] skipped: {reason}");
            return;
        }
        if build {
            super::run();
        }
        if !(warm || cold) {
            return;
        }

        let n_docs = supertable::n_docs();
        let infino_corpus = supertable::prepare_corpus(supertable::Modality::Sql);
        let infino_built = supertable::build_on_storage(supertable::Modality::Sql, &infino_corpus);
        let corpus = MmapTextCorpus::generate(n_docs, 1);
        let corpus_rows = corpus.rows();
        let rows = sql_rows(&corpus_rows);
        let cfg = SqlRunConfig {
            iters: WARM_ITERS,
            parallel: 1,
        };
        let (lance_warm, lance_index) = run_sql_with_index::<LanceS3SqlEngine>(
            cfg,
            &rows,
            infino_bench_utils::executors::sql::SQL_BATTERY,
        );

        let mut report = Report::load_plain("comparison-supertable-sql");
        if warm {
            let infino_warm = measure_infino_warm(&infino_built);
            let lance_warm_rows: Vec<_> =
                lance_warm.queries.iter().map(|q| (q.name, q.p50)).collect();
            emit_latency_comparison(
                &mut report,
                "comparison/supertable/sql/warm",
                format!(
                    "Supertable SQL comparison — warm queries ({} rows)",
                    fmt_count(n_docs)
                ),
                "warm = object-store table opened with a warm consumer/cache. Engines without this tier are omitted.",
                "warm",
                &infino_warm,
                &lance_warm_rows,
            );
        }
        if cold {
            let infino_cold = measure_infino_cold(&infino_built);
            let lance_cold = measure_lance_cold(&lance_index);
            emit_latency_comparison(
                &mut report,
                "comparison/supertable/sql/cold",
                format!(
                    "Supertable SQL comparison — cold queries ({} rows)",
                    fmt_count(n_docs)
                ),
                "Cold = fresh object-store read path per iteration; rebuild time is excluded.",
                "cold",
                &infino_cold,
                &lance_cold,
            );
        }
        report.save();

        if let Some(cleanup) = &infino_built.cleanup {
            tiers::cleanup_prefix(cleanup);
        }
    }

    fn open_infino_consumer(
        built: &supertable::IngestResult,
    ) -> (tempfile::TempDir, infino::supertable::Supertable) {
        let (cache_dir, cache) = tiers::fresh_supertable_search_cache(
            Arc::clone(&built.storage),
            Some(built.total_index_bytes),
        );
        let opts = tiers::consumer_options(
            supertable::options_for(supertable::Modality::Sql, None),
            Arc::clone(&built.storage),
            cache,
        );
        (cache_dir, tiers::open_consumer(opts))
    }

    fn measure_infino_warm(built: &supertable::IngestResult) -> Vec<(&'static str, Duration)> {
        let (_cache_dir, table) = open_infino_consumer(built);
        infino_bench_utils::executors::sql::SQL_BATTERY
            .iter()
            .map(|q| {
                let reader = table.reader();
                let _ = reader.query_sql(q.sql).expect("warmup infino sql");
                let mut samples = Vec::with_capacity(WARM_ITERS);
                for _ in 0..WARM_ITERS {
                    let t = Instant::now();
                    let batches = reader.query_sql(q.sql).expect("warm infino sql");
                    std::hint::black_box(batches);
                    samples.push(t.elapsed());
                }
                (q.name, p50(&mut samples))
            })
            .collect()
    }

    fn measure_infino_cold(built: &supertable::IngestResult) -> Vec<(&'static str, Duration)> {
        infino_bench_utils::executors::sql::SQL_BATTERY
            .iter()
            .map(|q| {
                let mut samples = Vec::with_capacity(COLD_ITERS);
                for _ in 0..COLD_ITERS {
                    let (cache_dir, table) = open_infino_consumer(built);
                    let t = Instant::now();
                    let batches = table.reader().query_sql(q.sql).expect("cold infino sql");
                    std::hint::black_box(batches);
                    samples.push(t.elapsed());
                    drop(table);
                    drop(cache_dir);
                }
                (q.name, p50(&mut samples))
            })
            .collect()
    }

    fn measure_lance_cold(
        index: &retrievalbench::lance::sql::LanceSqlIndex,
    ) -> Vec<(&'static str, Duration)> {
        infino_bench_utils::executors::sql::SQL_BATTERY
            .iter()
            .map(|q| {
                let mut samples = Vec::with_capacity(COLD_ITERS);
                for _ in 0..COLD_ITERS {
                    let ctx = index.cold_open();
                    let t = Instant::now();
                    let out = index.cold_query(&ctx, q.sql);
                    std::hint::black_box(out);
                    samples.push(t.elapsed());
                    drop(ctx);
                }
                (q.name, p50(&mut samples))
            })
            .collect()
    }
}
