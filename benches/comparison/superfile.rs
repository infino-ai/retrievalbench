// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Superfile-layer comparison benches grouped by modality.

pub mod fts {
    // SPDX-License-Identifier: Apache-2.0
    // SPDX-FileCopyrightText: Copyright The Infino Authors

    //! FTS comparison bench — drives infino, lancedb, and tantivy through the
    //! same `run_fts` driver and emits a single comparison section.

    use std::collections::HashMap;

    use infino_bench_utils::corpus::{MmapTextCorpus, parallel_writers, superfile_docs};
    use infino_bench_utils::executors::fts::FTS_BATTERY;
    use infino_bench_utils::superfile::fts::{FTS_COLUMN, K, WARM_ITERS};
    use infino_bench_utils::harness::{EngineFtsResult, InfinoFtsEngine, run_fts};
    use infino_bench_utils::markdown::{fmt_bandwidth, fmt_count, fmt_throughput, fmt_time};
    use infino_bench_utils::report::{Better, Block, Report, Section, metric, text};
    use infino_bench_utils::rss;

    use retrievalbench::{LanceFtsEngine, TantivyFtsEngine};

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

    pub fn run() {
        let n_docs = superfile_docs();
        eprintln!("[comparison-fts] generating {} docs...", fmt_count(n_docs));
        let corpus = MmapTextCorpus::generate(n_docs, 1);
        let docs = corpus.rows();
        let parallel = parallel_writers();

        let results: Vec<(&str, EngineFtsResult)> = vec![
            ("infino", run_fts::<InfinoFtsEngine>(FTS_COLUMN, &docs, FTS_BATTERY, K, WARM_ITERS, parallel)),
            ("lancedb", run_fts::<LanceFtsEngine>(FTS_COLUMN, &docs, FTS_BATTERY, K, WARM_ITERS, parallel)),
            ("tantivy", run_fts::<TantivyFtsEngine>(FTS_COLUMN, &docs, FTS_BATTERY, K, WARM_ITERS, parallel)),
        ];

        let input_bytes = corpus.total_bytes() as f64;
        let mut report = Report::load_plain("comparison-fts");

        // Index by (engine, writers) for build and (engine, query_name) for queries
        let build_map: HashMap<(&str, usize), _> = results
            .iter()
            .flat_map(|(name, res)| res.builds.iter().map(move |b| ((*name, b.writers), b)))
            .collect();
        let query_map: HashMap<(&str, &str), _> = results
            .iter()
            .flat_map(|(name, res)| res.queries.iter().map(move |q| ((*name, q.name), q)))
            .collect();

        let engines = ["infino", "lancedb", "tantivy"];
        let peers = ["lancedb", "tantivy"];

        // ── Build comparison blocks (one per metric) ────────────────────────
        let writer_counts: Vec<usize> = results[0].1.builds.iter().map(|b| b.writers).collect();

        let mut build_time_rows = Vec::new();
        let mut build_thr_rows = Vec::new();
        let mut build_bw_rows = Vec::new();
        let mut build_peak_rows = Vec::new();
        let mut build_med_rows = Vec::new();
        let mut build_p90_rows = Vec::new();

        for w in &writer_counts {
            let base = build_map.get(&("infino", *w)).expect("infino build");
            let base_secs = base.phase.wall.as_secs_f64();
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
                let secs = b.phase.wall.as_secs_f64();
                let ns = secs * 1e9;
                let thr = n_docs as f64 / secs;
                let bw = input_bytes / secs;

                time_row.push(metric(ns, fmt_time(ns), Better::Lower));
                thr_row.push(metric(thr, fmt_throughput(thr), Better::Higher));
                bw_row.push(metric(bw, fmt_bandwidth(bw), Better::Higher));
                peak_row.push(metric(
                    b.phase.rss.peak_rss_bytes as f64,
                    rss::fmt_bytes(b.phase.rss.peak_rss_bytes),
                    Better::Lower,
                ));
                med_row.push(metric(
                    b.phase.rss.median_rss_bytes as f64,
                    rss::fmt_bytes(b.phase.rss.median_rss_bytes),
                    Better::Lower,
                ));
                p90_row.push(metric(
                    b.phase.rss.p90_rss_bytes as f64,
                    rss::fmt_bytes(b.phase.rss.p90_rss_bytes),
                    Better::Lower,
                ));
            }

            for peer in &peers {
                let b = build_map.get(&(peer, *w)).expect("peer build");
                let secs = b.phase.wall.as_secs_f64();
                let ns = secs * 1e9;
                let thr = n_docs as f64 / secs;
                let bw = input_bytes / secs;

                time_row.push(metric(ns - base_ns, pct(ns, base_ns), Better::Lower));
                thr_row.push(metric(thr - base_thr, pct(thr, base_thr), Better::Higher));
                bw_row.push(metric(bw - base_bw, pct(bw, base_bw), Better::Higher));
                peak_row.push(metric(
                    b.phase.rss.peak_rss_bytes as f64 - base.phase.rss.peak_rss_bytes as f64,
                    pct(b.phase.rss.peak_rss_bytes as f64, base.phase.rss.peak_rss_bytes as f64),
                    Better::Lower,
                ));
                med_row.push(metric(
                    b.phase.rss.median_rss_bytes as f64 - base.phase.rss.median_rss_bytes as f64,
                    pct(b.phase.rss.median_rss_bytes as f64, base.phase.rss.median_rss_bytes as f64),
                    Better::Lower,
                ));
                p90_row.push(metric(
                    b.phase.rss.p90_rss_bytes as f64 - base.phase.rss.p90_rss_bytes as f64,
                    pct(b.phase.rss.p90_rss_bytes as f64, base.phase.rss.p90_rss_bytes as f64),
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
            "tantivy".into(),
            "lancedb Δ".into(),
            "tantivy Δ".into(),
        ];

        // ── Query comparison blocks (one per metric) ──────────────────────
        let mut query_lat_rows = Vec::new();
        let mut query_rss_rows = Vec::new();

        for qname in FTS_BATTERY.iter().map(|q| q.name) {
            let base = query_map.get(&("infino", qname)).expect("infino query");
            let base_ns = base.p50.as_secs_f64() * 1e9;
            let base_rss = base.rss.peak_rss_bytes as f64;

            let mut lat_row = vec![text(qname)];
            let mut rss_row = vec![text(qname)];

            for eng in &engines {
                let q = query_map.get(&(eng, qname)).expect("query stat");
                let ns = q.p50.as_secs_f64() * 1e9;
                lat_row.push(metric(ns, fmt_time(ns), Better::Lower));
                rss_row.push(metric(
                    q.rss.peak_rss_bytes as f64,
                    rss::fmt_bytes(q.rss.peak_rss_bytes),
                    Better::Lower,
                ));
            }

            for peer in &peers {
                let q = query_map.get(&(peer, qname)).expect("peer query");
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
            "tantivy".into(),
            "lancedb Δ".into(),
            "tantivy Δ".into(),
        ];

        report.emit(&Section {
            anchor: "comparison/fts".into(),
            title: format!("FTS comparison ({} docs)", fmt_count(n_docs)),
            note: "All engines driven through the same `run_fts` driver, corpus, and query battery. Δ is vs infino baseline.".into(),
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
}

pub mod vector {
    // SPDX-License-Identifier: Apache-2.0
    // SPDX-FileCopyrightText: Copyright The Infino Authors

    //! Vector comparison bench — drives infino and lancedb through the same
    //! `run_vector` build driver and the same recall-calibrated search
    //! protocol infino's own vector bench uses (`executors::vector`):
    //! per-engine `(probe, refine)` grid calibration against shared
    //! brute-force ground truth, reported at matched recall targets.

    use std::collections::HashMap;

    use infino_bench_utils::corpus::{self, parallel_writers};
    use infino_bench_utils::executors::vector as exec_vec;
    use infino_bench_utils::harness::{
        EngineVectorResult, InfinoVectorEngine, VectorEngine, VectorMetric, VectorQuery,
        VectorRunConfig, run_vector_with_index,
    };
    use infino_bench_utils::markdown::{fmt_bandwidth, fmt_count, fmt_throughput, fmt_time};
    use infino_bench_utils::report::{Better, Block, Report, Section, metric, text};
    use infino_bench_utils::rss;
    use infino_bench_utils::superfile::vector::{
        ground_truth_calibration, ground_truth_correctness, queries_calibration,
        queries_correctness, vectors,
    };

    use retrievalbench::LanceVectorEngine;

    const TOP_K: usize = 10;
    const VEC_COLUMN: &str = "v";
    const DEFAULT_NPROBE: usize = 8;
    const DEFAULT_RERANK_MULT: usize = 20;

    /// One row of the recall-calibrated search table for one engine.
    struct CalRow {
        label: String,
        point: Option<(usize, usize)>,
        recall: f32,
        p50_ns: f64,
    }

    /// The shared protocol, identical to infino's own vector bench: for
    /// each recall target, grid-calibrate `(probe, refine)` against the
    /// shared ground truth and time the cheapest qualifying point; then a
    /// `default` row at the user-facing defaults.
    fn calibrated_rows<R: exec_vec::VectorRead>(reader: &R, engine: &str) -> Vec<CalRow> {
        let qc = queries_calibration();
        let tc = ground_truth_calibration();
        let mut rows = Vec::new();
        for &target in exec_vec::RECALL_TARGETS {
            eprintln!("[comparison-vector] {engine}: calibrating recall@{target:.2}: grid over probes/refines ({} queries)...", qc.len());
            match exec_vec::calibrate(reader, VEC_COLUMN, qc, tc, target, TOP_K, "comparison-vector") {
                Some(c) => {
                    let t = exec_vec::measure_warm(reader, VEC_COLUMN, &qc[0], TOP_K, c.probe, c.refine);
                    rows.push(CalRow {
                        label: format!("{target:.2}"),
                        point: Some((c.probe, c.refine)),
                        recall: c.recall,
                        p50_ns: t.p50_ns,
                    });
                }
                None => rows.push(CalRow {
                    label: format!("{target:.2}"),
                    point: None,
                    recall: f32::NAN,
                    p50_ns: f64::NAN,
                }),
            }
        }
        let recall = exec_vec::mean_recall(
            reader, VEC_COLUMN, qc, tc, TOP_K, DEFAULT_NPROBE, DEFAULT_RERANK_MULT,
        );
        let t = exec_vec::measure_warm(
            reader, VEC_COLUMN, &qc[0], TOP_K, DEFAULT_NPROBE, DEFAULT_RERANK_MULT,
        );
        rows.push(CalRow {
            label: "default".into(),
            point: Some((DEFAULT_NPROBE, DEFAULT_RERANK_MULT)),
            recall,
            p50_ns: t.p50_ns,
        });
        rows
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

    pub fn run() {
        let n_docs = corpus::superfile_docs();
        let vecs = vectors();
        eprintln!(
            "[comparison-vector] {} docs × dim={}, recall-calibrated protocol",
            fmt_count(n_docs),
            corpus::DIM
        );

        let cfg = VectorRunConfig {
            column: VEC_COLUMN,
            dim: corpus::DIM,
            metric: VectorMetric::Cosine,
            k: TOP_K,
            iters: exec_vec::CALIBRATION_P50_ITERS,
            parallel: parallel_writers(),
        };

        // Build through the shared driver; keep both indexes alive for the
        // calibrated search protocol below (no fixed-default query battery —
        // search rows come from the recall-target calibration).
        let no_queries: Vec<VectorQuery<'_>> = Vec::new();
        let (infino_res, mut infino_idx) =
            run_vector_with_index::<InfinoVectorEngine>(cfg, vecs, &no_queries);
        let (lance_res, mut lance_idx) =
            run_vector_with_index::<LanceVectorEngine>(cfg, vecs, &no_queries);
        let results: Vec<(&str, EngineVectorResult)> =
            vec![("infino", infino_res), ("lancedb", lance_res)];

        // Correctness gate (shared battery + thresholds) — reported, and
        // asserted for infino exactly like its own bench.
        let qcorr = queries_correctness();
        let tcorr = ground_truth_correctness();
        let infino_gate = exec_vec::mean_recall(
            infino_idx.reader(), VEC_COLUMN, qcorr, tcorr, TOP_K,
            exec_vec::CORRECTNESS_NPROBE, exec_vec::CORRECTNESS_RERANK_MULT,
        );
        let lance_gate = exec_vec::mean_recall(
            &lance_idx, VEC_COLUMN, qcorr, tcorr, TOP_K,
            exec_vec::CORRECTNESS_NPROBE, exec_vec::CORRECTNESS_RERANK_MULT,
        );
        eprintln!(
            "[comparison-vector] correctness recall@{TOP_K}: infino={infino_gate:.3} lancedb={lance_gate:.3} (floor {:.2})",
            exec_vec::CORRECTNESS_RECALL_FLOOR
        );
        assert!(
            infino_gate >= exec_vec::CORRECTNESS_RECALL_FLOOR,
            "infino correctness gate failed: {infino_gate:.3}"
        );

        let infino_rows = calibrated_rows(infino_idx.reader(), "infino");
        let lance_rows = calibrated_rows(&lance_idx, "lancedb");

        let input_bytes = (n_docs * corpus::DIM * std::mem::size_of::<f32>()) as f64;
        let mut report = Report::load_plain("comparison-vector");

        let build_map: HashMap<(&str, usize), _> = results
            .iter()
            .flat_map(|(name, res)| res.builds.iter().map(move |b| ((*name, b.writers), b)))
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

        // Recall-calibrated search table: one row per recall target plus
        // the defaults row, each engine at its own cheapest qualifying
        // (probe, refine) point — latency compared at matched recall.
        let mut recall_rows = Vec::new();
        for (inf, lan) in infino_rows.iter().zip(&lance_rows) {
            let fmt_point = |p: Option<(usize, usize)>| match p {
                Some((probe, refine)) => format!("p={probe}, r={refine}"),
                None => "—".into(),
            };
            let fmt_recall = |r: f32| if r.is_nan() { "—".into() } else { format!("{r:.3}") };
            let mut row = vec![
                text(inf.label.clone()),
                text(fmt_point(inf.point)),
                text(fmt_recall(inf.recall)),
                metric(inf.p50_ns, fmt_time(inf.p50_ns), Better::Lower),
                text(fmt_point(lan.point)),
                text(fmt_recall(lan.recall)),
            ];
            if lan.p50_ns.is_nan() {
                row.push(text(String::from("—")));
                row.push(text(String::from("—")));
            } else {
                row.push(metric(lan.p50_ns, fmt_time(lan.p50_ns), Better::Lower));
                row.push(metric(
                    lan.p50_ns - inf.p50_ns,
                    pct(lan.p50_ns, inf.p50_ns),
                    Better::Lower,
                ));
            }
            recall_rows.push(row);
        }

        let recall_headers: Vec<String> = vec![
            "Recall target".into(),
            "infino (p, r)".into(),
            "infino recall".into(),
            "infino p50".into(),
            "lancedb (p, r)".into(),
            "lancedb recall".into(),
            "lancedb p50".into(),
            "lancedb Δ".into(),
        ];

        report.emit(&Section {
            anchor: "comparison/vector".into(),
            title: format!("Vector comparison ({} docs × dim={})", fmt_count(n_docs), corpus::DIM),
            note: format!(
                "All engines driven through the same build driver, corpus, ground truth, and \
                 recall-calibration grid (infino's own vector protocol). Search rows compare \
                 latency at matched recall: each engine's lowest-p50 (probe, refine) point \
                 clearing the target. Correctness gate recall@{TOP_K}: infino {infino_gate:.3}, \
                 lancedb {lance_gate:.3}. Δ is vs infino p50."
            ),
            blocks: vec![
                Block { subtitle: "Build — Time".into(), headers: build_headers.clone(), rows: build_time_rows },
                Block { subtitle: "Build — Throughput".into(), headers: build_headers.clone(), rows: build_thr_rows },
                Block { subtitle: "Build — Bandwidth".into(), headers: build_headers.clone(), rows: build_bw_rows },
                Block { subtitle: "Build — Peak RSS".into(), headers: build_headers.clone(), rows: build_peak_rows },
                Block { subtitle: "Build — Median RSS".into(), headers: build_headers.clone(), rows: build_med_rows },
                Block { subtitle: "Build — P90 RSS".into(), headers: build_headers.clone(), rows: build_p90_rows },
                Block { subtitle: "Search — recall-calibrated (warm p50)".into(), headers: recall_headers, rows: recall_rows },
            ],
        });

        report.save();

        InfinoVectorEngine::close(&mut infino_idx);
        InfinoVectorEngine::delete(infino_idx);
        LanceVectorEngine::close(&mut lance_idx);
        LanceVectorEngine::delete(lance_idx);
    }
}

pub mod sql {
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
    use infino_bench_utils::executors::sql::{ITERS, SQL_BATTERY};
    use infino_bench_utils::superfile::sql::sql_rows;

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

    pub fn run() {
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

        let mut report = Report::load_plain("comparison-sql");

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
}

pub fn run() {
    fts::run();
    vector::run();
    sql::run();
}

#[allow(dead_code)]
fn main() {
    run();
}
