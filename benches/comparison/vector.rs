// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Vector comparison bench — drives infino and lancedb through the same
//! `run_vector` driver and emits a single comparison section.

use std::collections::HashMap;

use infino_bench_utils::corpus::{self, parallel_writers};
use infino_bench_utils::harness::{
    EngineVectorResult, InfinoVectorEngine, VectorMetric, VectorQuery,
    VectorRunConfig, VectorSearch, run_vector,
};
use infino_bench_utils::markdown::{fmt_bandwidth, fmt_count, fmt_throughput, fmt_time};
use infino_bench_utils::report::{Better, Block, Report, Section, metric, text};
use infino_bench_utils::rss;
use infino_bench_utils::vector_superfile::{queries_correctness, vectors};

use retrievalbench::LanceVectorEngine;

const TOP_K: usize = 10;
const VEC_COLUMN: &str = "v";
const CALIBRATION_P50_ITERS: usize = 7;
const DEFAULT_NPROBE: usize = 8;
const DEFAULT_RERANK_MULT: usize = 20;

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

fn main() {
    let n_docs = corpus::superfile_docs();
    let vecs = vectors();
    let queries = queries_correctness();
    eprintln!("[comparison-vector] {} docs × dim={}, {} queries", fmt_count(n_docs), corpus::DIM, queries.len());

    let qs: Vec<VectorQuery<'_>> = queries
        .iter()
        .enumerate()
        .map(|(i, q)| VectorQuery {
            name: Box::leak(format!("q{i}").into_boxed_str()),
            vector: q,
            search: VectorSearch {
                nprobe: DEFAULT_NPROBE,
                rerank_mult: DEFAULT_RERANK_MULT,
            },
        })
        .collect();

    let cfg = VectorRunConfig {
        column: VEC_COLUMN,
        dim: corpus::DIM,
        metric: VectorMetric::Cosine,
        k: TOP_K,
        iters: CALIBRATION_P50_ITERS,
        parallel: parallel_writers(),
    };

    let results: Vec<(&str, EngineVectorResult)> = vec![
        ("infino", run_vector::<InfinoVectorEngine>(cfg, vecs, &qs)),
        ("lancedb", run_vector::<LanceVectorEngine>(cfg, vecs, &qs)),
    ];

    let input_bytes = (n_docs * corpus::DIM * std::mem::size_of::<f32>()) as f64;
    let mut report = Report::load("comparison-vector");

    let build_map: HashMap<(&str, usize), _> = results
        .iter()
        .flat_map(|(name, res)| res.builds.iter().map(move |b| ((*name, b.writers), b)))
        .collect();
    let query_map: HashMap<(&str, &str), _> = results
        .iter()
        .flat_map(|(name, res)| res.queries.iter().map(move |q| ((*name, q.name), q)))
        .collect();

    let engines = ["infino", "lancedb"];
    let peers = ["lancedb"];

    let writer_counts: Vec<usize> = results[0].1.builds.iter().map(|b| b.writers).collect();

    let mut build_time_rows = Vec::new();
    let mut build_thr_rows = Vec::new();
    let mut build_bw_rows = Vec::new();
    let mut build_peak_rows = Vec::new();
    let mut build_med_rows = Vec::new();
    let mut build_p90_rows = Vec::new();

    for w in &writer_counts {
        let base = build_map.get(&("infino", *w)).expect("infino build");
        let base_secs = base.wall.as_secs_f64();
        let base_ns = base_secs * 1e9;
        let base_thr = n_docs as f64 / base_secs;
        let base_bw = input_bytes / base_secs;

        let label = text(if *w == 1 { "1 writer".into() } else { format!("{w} writers") });
        let mut time_row = vec![label];
        let mut thr_row = vec![text(if *w == 1 { "1 writer".into() } else { format!("{w} writers") })];
        let mut bw_row = vec![text(if *w == 1 { "1 writer".into() } else { format!("{w} writers") })];
        let mut peak_row = vec![text(if *w == 1 { "1 writer".into() } else { format!("{w} writers") })];
        let mut med_row = vec![text(if *w == 1 { "1 writer".into() } else { format!("{w} writers") })];
        let mut p90_row = vec![text(if *w == 1 { "1 writer".into() } else { format!("{w} writers") })];

        for eng in &engines {
            let b = build_map.get(&(eng, *w)).expect("build stat");
            let secs = b.wall.as_secs_f64();
            let ns = secs * 1e9;
            let thr = n_docs as f64 / secs;
            let bw = input_bytes / secs;

            time_row.push(metric(ns, fmt_time(ns), Better::Lower));
            thr_row.push(metric(thr, fmt_throughput(thr), Better::Higher));
            bw_row.push(metric(bw, fmt_bandwidth(bw), Better::Higher));
            peak_row.push(metric(
                b.rss.peak_rss_bytes as f64,
                rss::fmt_bytes(b.rss.peak_rss_bytes),
                Better::Lower,
            ));
            med_row.push(metric(
                b.rss.median_rss_bytes as f64,
                rss::fmt_bytes(b.rss.median_rss_bytes),
                Better::Lower,
            ));
            p90_row.push(metric(
                b.rss.p90_rss_bytes as f64,
                rss::fmt_bytes(b.rss.p90_rss_bytes),
                Better::Lower,
            ));
        }

        for peer in &peers {
            let b = build_map.get(&(peer, *w)).expect("peer build");
            let secs = b.wall.as_secs_f64();
            let ns = secs * 1e9;
            let thr = n_docs as f64 / secs;
            let bw = input_bytes / secs;

            time_row.push(metric(ns - base_ns, pct(ns, base_ns), Better::Lower));
            thr_row.push(metric(thr - base_thr, pct(thr, base_thr), Better::Higher));
            bw_row.push(metric(bw - base_bw, pct(bw, base_bw), Better::Higher));
            peak_row.push(metric(
                b.rss.peak_rss_bytes as f64 - base.rss.peak_rss_bytes as f64,
                pct(b.rss.peak_rss_bytes as f64, base.rss.peak_rss_bytes as f64),
                Better::Lower,
            ));
            med_row.push(metric(
                b.rss.median_rss_bytes as f64 - base.rss.median_rss_bytes as f64,
                pct(b.rss.median_rss_bytes as f64, base.rss.median_rss_bytes as f64),
                Better::Lower,
            ));
            p90_row.push(metric(
                b.rss.p90_rss_bytes as f64 - base.rss.p90_rss_bytes as f64,
                pct(b.rss.p90_rss_bytes as f64, base.rss.p90_rss_bytes as f64),
                Better::Lower,
            ));
        }

        build_time_rows.push(time_row);
        build_thr_rows.push(thr_row);
        build_bw_rows.push(bw_row);
        build_peak_rows.push(peak_row);
        build_med_rows.push(med_row);
        build_p90_rows.push(p90_row);
    }

    let build_headers = vec![
        "Config".into(),
        "infino".into(),
        "lancedb".into(),
        "lancedb Δ".into(),
    ];

    let mut query_lat_rows = Vec::new();
    let mut query_rss_rows = Vec::new();

    for qname in queries.iter().enumerate().map(|(i, _)| format!("q{i}")) {
        let base = query_map.get(&("infino", qname.as_str())).expect("infino query");
        let base_ns = base.p50.as_secs_f64() * 1e9;
        let base_rss = base.rss.peak_rss_bytes as f64;

        let mut lat_row = vec![text(qname.clone())];
        let mut rss_row = vec![text(qname.clone())];

        for eng in &engines {
            let q = query_map.get(&(eng, qname.as_str())).expect("query stat");
            let ns = q.p50.as_secs_f64() * 1e9;
            lat_row.push(metric(ns, fmt_time(ns), Better::Lower));
            rss_row.push(metric(
                q.rss.peak_rss_bytes as f64,
                rss::fmt_bytes(q.rss.peak_rss_bytes),
                Better::Lower,
            ));
        }

        for peer in &peers {
            let q = query_map.get(&(peer, qname.as_str())).expect("peer query");
            let ns = q.p50.as_secs_f64() * 1e9;
            let rss = q.rss.peak_rss_bytes as f64;
            lat_row.push(metric(ns - base_ns, pct(ns, base_ns), Better::Lower));
            rss_row.push(metric(rss - base_rss, pct(rss, base_rss), Better::Lower));
        }

        query_lat_rows.push(lat_row);
        query_rss_rows.push(rss_row);
    }

    let query_headers = vec![
        "Query".into(),
        "infino".into(),
        "lancedb".into(),
        "lancedb Δ".into(),
    ];

    report.emit(&Section {
        anchor: "comparison/vector".into(),
        title: format!("Vector comparison ({} docs × dim={})", fmt_count(n_docs), corpus::DIM),
        note: "All engines driven through the same `run_vector` driver, corpus, and query set. Δ is vs infino baseline.".into(),
        blocks: vec![
            Block { subtitle: "Build — Time".into(), headers: build_headers.clone(), rows: build_time_rows },
            Block { subtitle: "Build — Throughput".into(), headers: build_headers.clone(), rows: build_thr_rows },
            Block { subtitle: "Build — Bandwidth".into(), headers: build_headers.clone(), rows: build_bw_rows },
            Block { subtitle: "Build — Peak RSS".into(), headers: build_headers.clone(), rows: build_peak_rows },
            Block { subtitle: "Build — Median RSS".into(), headers: build_headers.clone(), rows: build_med_rows },
            Block { subtitle: "Build — P90 RSS".into(), headers: build_headers.clone(), rows: build_p90_rows },
            Block { subtitle: "Query — Latency".into(), headers: query_headers.clone(), rows: query_lat_rows },
            Block { subtitle: "Query — Peak RSS".into(), headers: query_headers.clone(), rows: query_rss_rows },
        ],
    });

    report.save();
}
