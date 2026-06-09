// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Supertable object-store comparison bench.
//!
//! Mirrors Infino's `supertable_all` ingest benchmark shape and uses the
//! existing Infino supertable bench utilities as the baseline source of truth.

use infino_bench_utils::corpus::{self, MmapTextCorpus, MmapVectorCorpus};
use infino_bench_utils::harness::{
    FtsQuery, SqlQuery, SqlRunConfig, VectorMetric, VectorQuery, VectorRunConfig, run_fts,
    run_sql, run_vector,
};
use infino_bench_utils::ingest::supertable::{self, N_COMMIT_CHUNKS, TEXT_COLUMN, VEC_COLUMN};
use infino_bench_utils::markdown::{fmt_count, fmt_throughput, fmt_time};
use infino_bench_utils::report::{Better, Block, Cell, Report, Section, metric, text};
use infino_bench_utils::rss;
use infino_bench_utils::sql_bench::sql_rows;
use infino_bench_utils::supertable_bench::{
    handle_shape_child_from_env, ingest_row, run_ingest_shapes_isolated,
};
use infino_bench_utils::tiers;
use retrievalbench::{LanceS3FtsEngine, LanceS3SqlEngine, LanceS3VectorEngine};

const EMPTY_FTS_QUERIES: &[FtsQuery] = &[];
const EMPTY_VECTOR_QUERIES: &[VectorQuery<'_>] = &[];
const EMPTY_SQL_QUERIES: &[SqlQuery] = &[];

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
    let throughput = if secs > 0.0 { n_docs as f64 / secs } else { 0.0 };
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
    let throughput = if secs > 0.0 { n_docs as f64 / secs } else { 0.0 };
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
    let throughput = if secs > 0.0 { n_docs as f64 / secs } else { 0.0 };
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

fn main() {
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
        N_COMMIT_CHUNKS
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

    let mut report = Report::load("comparison-supertable");
    report.emit(&Section {
        anchor: "comparison/supertable/ingest".into(),
        title: format!(
            "Supertable comparison — ingest, multi-segment / object-store ({} docs × dim={}, {} commits)",
            fmt_count(n_docs),
            infino_bench_utils::corpus::DIM,
            N_COMMIT_CHUNKS
        ),
        note: "Infino baseline rows are produced by the same isolated shape measurement used by `supertable_all`: `SupertableWriter::append` + `commit` to object storage, one subprocess per shape. Peer rows use existing comparison drivers with public object-store configuration; LanceDB FTS/vector/SQL rows are driven by `run_fts`/`run_vector`/`run_sql` with S3-configured adapters.".into(),
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
