// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! SQL comparison bench — drives infino and lancedb through the same
//! `run_sql` driver and emits a single comparison section.

use std::collections::HashMap;

use infino_bench_utils::corpus::{MmapTextCorpus, parallel_writers, superfile_docs};
use infino_bench_utils::harness::{EngineSqlResult, InfinoSqlEngine, SqlRunConfig, run_sql};
use infino_bench_utils::markdown::{fmt_count, fmt_time};
use infino_bench_utils::report::{Better, Block, Report, Section, metric, text};
use infino_bench_utils::rss;
use infino_bench_utils::sql_bench::{ITERS, SQL_BATTERY, sql_rows};

use retrievalbench::LanceSqlEngine;

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
    let n_docs = superfile_docs();
    eprintln!("[comparison-sql] generating {} docs...", fmt_count(n_docs));
    let corpus = MmapTextCorpus::generate(n_docs, 1);
    let corpus_rows = corpus.rows();
    let rows = sql_rows(&corpus_rows);
    let parallel = parallel_writers();

    let cfg = SqlRunConfig {
        iters: ITERS,
        parallel,
    };

    let results: Vec<(&str, EngineSqlResult)> = vec![
        ("infino", run_sql::<InfinoSqlEngine>(cfg, &rows, SQL_BATTERY)),
        ("lancedb", run_sql::<LanceSqlEngine>(cfg, &rows, SQL_BATTERY)),
    ];

    let mut report = Report::load("comparison-sql");

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
    let mut build_peak_rows = Vec::new();
    let mut build_med_rows = Vec::new();
    let mut build_p90_rows = Vec::new();

    for w in &writer_counts {
        let base = build_map.get(&("infino", *w)).expect("infino build");
        let base_secs = base.wall.as_secs_f64();
        let base_ns = base_secs * 1e9;

        let label = text(if *w == 1 { "1 writer".into() } else { format!("{w} writers") });
        let mut time_row = vec![label];
        let mut peak_row = vec![text(if *w == 1 { "1 writer".into() } else { format!("{w} writers") })];
        let mut med_row = vec![text(if *w == 1 { "1 writer".into() } else { format!("{w} writers") })];
        let mut p90_row = vec![text(if *w == 1 { "1 writer".into() } else { format!("{w} writers") })];

        for eng in &engines {
            let b = build_map.get(&(eng, *w)).expect("build stat");
            let secs = b.wall.as_secs_f64();
            let ns = secs * 1e9;

            time_row.push(metric(ns, fmt_time(ns), Better::Lower));
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

            time_row.push(metric(ns - base_ns, pct(ns, base_ns), Better::Lower));
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
    let mut query_rows_rows = Vec::new();

    for qname in SQL_BATTERY.iter().map(|q| q.name) {
        let base = query_map.get(&("infino", qname)).expect("infino query");
        let base_ns = base.p50.as_secs_f64() * 1e9;
        let base_rss = base.rss.peak_rss_bytes as f64;

        let mut lat_row = vec![text(qname)];
        let mut rss_row = vec![text(qname)];
        let mut rows_row = vec![text(qname)];

        for eng in &engines {
            let q = query_map.get(&(eng, qname)).expect("query stat");
            let ns = q.p50.as_secs_f64() * 1e9;
            lat_row.push(metric(ns, fmt_time(ns), Better::Lower));
            rss_row.push(metric(
                q.rss.peak_rss_bytes as f64,
                rss::fmt_bytes(q.rss.peak_rss_bytes),
                Better::Lower,
            ));
            rows_row.push(text(format!("{}", q.rows)));
        }

        for peer in &peers {
            let q = query_map.get(&(peer, qname)).expect("peer query");
            let ns = q.p50.as_secs_f64() * 1e9;
            let rss = q.rss.peak_rss_bytes as f64;
            lat_row.push(metric(ns - base_ns, pct(ns, base_ns), Better::Lower));
            rss_row.push(metric(rss - base_rss, pct(rss, base_rss), Better::Lower));
            rows_row.push(text(format!("{}", q.rows)));
        }

        query_lat_rows.push(lat_row);
        query_rss_rows.push(rss_row);
        query_rows_rows.push(rows_row);
    }

    let query_headers = vec![
        "Query".into(),
        "infino".into(),
        "lancedb".into(),
        "lancedb Δ".into(),
    ];

    report.emit(&Section {
        anchor: "comparison/sql".into(),
        title: format!("SQL comparison ({} docs)", fmt_count(n_docs)),
        note: "All engines driven through the same `run_sql` driver, corpus, and query battery. Δ is vs infino baseline.".into(),
        blocks: vec![
            Block { subtitle: "Build — Time".into(), headers: build_headers.clone(), rows: build_time_rows },
            Block { subtitle: "Build — Peak RSS".into(), headers: build_headers.clone(), rows: build_peak_rows },
            Block { subtitle: "Build — Median RSS".into(), headers: build_headers.clone(), rows: build_med_rows },
            Block { subtitle: "Build — P90 RSS".into(), headers: build_headers.clone(), rows: build_p90_rows },
            Block { subtitle: "Query — Latency".into(), headers: query_headers.clone(), rows: query_lat_rows },
            Block { subtitle: "Query — Peak RSS".into(), headers: query_headers.clone(), rows: query_rss_rows },
            Block { subtitle: "Query — Rows returned".into(), headers: query_headers.clone(), rows: query_rows_rows },
        ],
    });

    report.save();
}
