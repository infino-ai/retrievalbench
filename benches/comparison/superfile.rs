// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Superfile-layer comparison benches grouped by modality.

pub mod fts {
    // SPDX-License-Identifier: Apache-2.0
    // SPDX-FileCopyrightText: Copyright The Infino Authors

    //! FTS comparison bench — drives infino and tantivy through the
    //! same `run_fts` driver and emits a single comparison section.

    use std::collections::HashMap;

    use infino_bench_utils::corpus::{MmapTextCorpus, parallel_writers, superfile_docs};
    use infino_bench_utils::executors::fts::FTS_BATTERY;
    use infino_bench_utils::harness::{EngineFtsResult, InfinoFtsEngine, run_fts};
    use infino_bench_utils::markdown::{fmt_bandwidth, fmt_count, fmt_throughput, fmt_time};
    use infino_bench_utils::report::{Better, Block, Report, Section, metric, text};
    use infino_bench_utils::rss;
    use infino_bench_utils::superfile::fts::{FTS_COLUMN, K, WARM_ITERS};

    use retrievalbench::TantivyFtsEngine;

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
            (
                "infino",
                run_fts::<InfinoFtsEngine>(FTS_COLUMN, &docs, FTS_BATTERY, K, WARM_ITERS, parallel),
            ),
            (
                "tantivy",
                run_fts::<TantivyFtsEngine>(
                    FTS_COLUMN,
                    &docs,
                    FTS_BATTERY,
                    K,
                    WARM_ITERS,
                    parallel,
                ),
            ),
        ];

        let input_bytes = corpus.total_bytes() as f64;
        let mut report = Report::load("comparison-fts");

        // Index by (engine, writers) for build and (engine, query_name) for queries
        let build_map: HashMap<(&str, usize), _> = results
            .iter()
            .flat_map(|(name, res)| res.builds.iter().map(move |b| ((*name, b.writers), b)))
            .collect();
        let query_map: HashMap<(&str, &str), _> = results
            .iter()
            .flat_map(|(name, res)| res.queries.iter().map(move |q| ((*name, q.name), q)))
            .collect();

        let engines = ["infino", "tantivy"];
        let peers = ["tantivy"];

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

            let label = text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            });
            let mut time_row = vec![label];
            let mut thr_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];
            let mut bw_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];
            let mut peak_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];
            let mut med_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];
            let mut p90_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];

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
                    pct(
                        b.phase.rss.peak_rss_bytes as f64,
                        base.phase.rss.peak_rss_bytes as f64,
                    ),
                    Better::Lower,
                ));
                med_row.push(metric(
                    b.phase.rss.median_rss_bytes as f64 - base.phase.rss.median_rss_bytes as f64,
                    pct(
                        b.phase.rss.median_rss_bytes as f64,
                        base.phase.rss.median_rss_bytes as f64,
                    ),
                    Better::Lower,
                ));
                p90_row.push(metric(
                    b.phase.rss.p90_rss_bytes as f64 - base.phase.rss.p90_rss_bytes as f64,
                    pct(
                        b.phase.rss.p90_rss_bytes as f64,
                        base.phase.rss.p90_rss_bytes as f64,
                    ),
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
            "tantivy".into(),
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
            "tantivy".into(),
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

    //! Vector comparison bench — drives infino through the same
    //! `run_vector` build driver and the same default-serving search
    //! protocol infino's own vector bench uses (`exec_vec::run_search`,
    //! grid off): each engine at its own shipped default configuration,
    //! recall measured on the shared brute-force oracle. The recall-target
    //! calibration grid is a retired legacy diagnostic behind
    //! [`RUN_CALIBRATION_GRID`].

    use std::collections::HashMap;
    use std::time::Duration;

    use infino::superfile::reader::VectorSearchOptions;
    use infino_bench_utils::corpus::{self, parallel_writers};
    use infino_bench_utils::cpu;
    use infino_bench_utils::executors::vector as exec_vec;
    use infino_bench_utils::harness::{
        EngineVectorResult, InfinoVectorEngine, InfinoVectorIndex, VectorEngine, VectorMetric,
        VectorQuery, VectorRunConfig, VectorSearch, run_vector_with_index,
    };
    use infino_bench_utils::markdown::{fmt_bandwidth, fmt_count, fmt_throughput, fmt_time};
    use infino_bench_utils::report::{Better, Block, Report, Section, metric, text};
    use infino_bench_utils::rss::{self, PeakSampler};
    use infino_bench_utils::superfile::vector::{
        ground_truth_calibration, ground_truth_correctness, queries_calibration,
        queries_correctness, vectors,
    };

    const TOP_K: usize = 10;
    const VEC_COLUMN: &str = "v";
    /// Recall-target calibration grid — off by default, mirroring infino's
    /// own benches: the shipped protocol measures each engine at its
    /// default serving configuration (floor-gated for infino, reported for
    /// peers). The grid is a legacy tuning diagnostic.
    pub(crate) const RUN_CALIBRATION_GRID: bool = false;

    /// `run_search` cold-opener placeholder for warm-only cells
    /// (`include_cold = false` ⇒ never called).
    pub(crate) struct NoCold;

    impl exec_vec::VectorRead for NoCold {
        fn topk_global(
            &self,
            _column: &str,
            _query: &[f32],
            _k: usize,
            _nprobe: usize,
            _rerank: usize,
        ) -> Vec<(u32, f32)> {
            unreachable!("cold tier is disabled for this cell")
        }
    }

    /// One compact side-by-side block from each engine's `default` row:
    /// recall, warm p50, cold open/search, and warm Δ vs the first
    /// (baseline) engine. Re-renders numbers the per-engine tables
    /// already measured — never re-measures.
    pub(crate) fn emit_default_comparison(
        report: &mut Report,
        anchor: &str,
        title: String,
        engines: &[(&str, &[exec_vec::RecallRow])],
        include_warm: bool,
        include_cold: bool,
    ) {
        const NS_PER_SEC: f64 = 1e9;
        fn default_row(rows: &[exec_vec::RecallRow]) -> Option<&exec_vec::RecallRow> {
            rows.iter().find(|r| r.target == "default")
        }
        let baseline_warm_ns = engines
            .first()
            .and_then(|(_, rows)| default_row(rows))
            .and_then(|r| r.warm.as_ref())
            .map(|t| t.warm.p50.as_secs_f64() * NS_PER_SEC);
        let mut rows = Vec::new();
        for (i, (name, engine_rows)) in engines.iter().enumerate() {
            for r in engine_rows.iter() {
                let label = if r.target == "default" {
                    (*name).to_string()
                } else {
                    format!("{name} ({})", r.target)
                };
                let mut row = vec![text(label), text(r.params.clone()), text(r.recall.clone())];
                if include_warm {
                    match &r.warm {
                        Some(t) => {
                            let ns = t.warm.p50.as_secs_f64() * NS_PER_SEC;
                            row.push(metric(ns, fmt_time(ns), Better::Lower));
                        }
                        None => row.push(text(String::from("—"))),
                    }
                }
                if include_cold {
                    match &r.cold {
                        Some(c) => {
                            let open_ns = c.open.as_secs_f64() * NS_PER_SEC;
                            let search_ns = c.search.as_secs_f64() * NS_PER_SEC;
                            row.push(metric(open_ns, fmt_time(open_ns), Better::Lower));
                            row.push(metric(search_ns, fmt_time(search_ns), Better::Lower));
                        }
                        None => {
                            row.push(text(String::from("—")));
                            row.push(text(String::from("—")));
                        }
                    }
                }
                if include_warm {
                    match (i, baseline_warm_ns, &r.warm) {
                        (0, _, _) => row.push(text(String::from("baseline"))),
                        (_, Some(base_ns), Some(t)) => {
                            let ns = t.warm.p50.as_secs_f64() * NS_PER_SEC;
                            row.push(metric(ns - base_ns, pct(ns, base_ns), Better::Lower));
                        }
                        _ => row.push(text(String::from("—"))),
                    }
                }
                rows.push(row);
            }
        }
        let mut headers = vec![
            "Engine".to_string(),
            "Search parameters".to_string(),
            "recall".to_string(),
        ];
        if include_warm {
            headers.push("warm p50".into());
        }
        if include_cold {
            headers.push("cold open".into());
            headers.push("cold search".into());
        }
        if include_warm {
            headers.push("warm Δ".into());
        }
        report.emit(&Section {
            anchor: anchor.into(),
            title,
            note: "Each engine at its own shipped default serving configuration — no \
                   harness-tuned search parameters. Recall is measured on the shared \
                   brute-force oracle; Δ is vs the first engine's warm p50."
                .into(),
            blocks: vec![Block {
                subtitle: "Search — default serving".into(),
                headers,
                rows,
            }],
        });
    }

    /// Superfile save/load is real (`finish()` bytes / `SuperfileReader::open`).
    /// Insert/remove is not — a sealed superfile is never mutated.
    fn emit_superfile_save_load(
        report: &mut Report,
        index: &InfinoVectorIndex,
        query: &[f32],
        cfg: VectorRunConfig<'_>,
        n_docs: usize,
    ) {
        const NS_PER_SEC: f64 = 1e9;
        fn timed(f: impl FnOnce()) -> (Duration, u64) {
            let sampler = PeakSampler::start_default();
            let ((), wall, _) = cpu::timed(f);
            (wall, sampler.stop_stats().peak_rss_bytes)
        }
        let search = VectorSearch {
            nprobe: exec_vec::ENGINE_DEFAULT,
            rerank_mult: exec_vec::ENGINE_DEFAULT,
        };

        let (save_wall, save_rss) = timed(|| {
            let bytes = InfinoVectorEngine::save(index).expect("infino superfile save");
            std::hint::black_box(bytes.len());
        });
        let saved = InfinoVectorEngine::save(index).expect("infino superfile save bytes");
        let (load_wall, load_rss) = timed(|| {
            let loaded =
                InfinoVectorEngine::load(cfg.column, cfg.dim, cfg.metric, &saved).expect("load");
            std::hint::black_box(loaded.bytes().len());
        });
        let loaded = InfinoVectorEngine::load(cfg.column, cfg.dim, cfg.metric, &saved)
            .expect("load for search");
        let (first_wall, first_rss) = timed(|| {
            let hits = InfinoVectorEngine::read(&loaded, query, cfg.k, search);
            std::hint::black_box(hits);
        });

        let row = |label: &str, wall: Duration, peak: u64| {
            let ns = wall.as_secs_f64() * NS_PER_SEC;
            vec![
                text(label),
                metric(ns, fmt_time(ns), Better::Lower),
                metric(peak as f64, rss::fmt_bytes(peak), Better::Lower),
            ]
        };

        report.emit(&Section {
            anchor: "comparison/superfile/vector/save-load".into(),
            title: format!(
                "Superfile save/load — infino ({} docs × dim={})",
                fmt_count(n_docs),
                cfg.dim
            ),
            note: "`finish()` already returns final bytes; `SuperfileReader::open` reopens \
                   them. There is no superfile insert/remove cell — new rows are table \
                   `append`, measured separately."
                .into(),
            blocks: vec![Block {
                subtitle: "infino superfile".into(),
                headers: vec!["Op".into(), "Wall".into(), "Peak RSS".into()],
                rows: vec![
                    row("save", save_wall, save_rss),
                    row("load", load_wall, load_rss),
                    row("load → first search", first_wall, first_rss),
                ],
            }],
        });
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
            corpus::dim()
        );

        let cfg = VectorRunConfig {
            column: VEC_COLUMN,
            dim: corpus::dim(),
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
        let results: Vec<(&str, EngineVectorResult)> = vec![("infino", infino_res)];

        let input_bytes = (n_docs * corpus::dim() * std::mem::size_of::<f32>()) as f64;
        let mut report = Report::load("comparison-vector");

        let build_map: HashMap<(&str, usize), _> = results
            .iter()
            .flat_map(|(name, res)| res.builds.iter().map(move |b| ((*name, b.writers), b)))
            .collect();

        let engines = ["infino"];
        let peers: [&str; 0] = [];

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

            let label = text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            });
            let mut time_row = vec![label];
            let mut thr_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];
            let mut bw_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];
            let mut peak_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];
            let mut med_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];
            let mut p90_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];

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
                    pct(
                        b.rss.median_rss_bytes as f64,
                        base.rss.median_rss_bytes as f64,
                    ),
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

        let build_headers = vec!["Config".into(), "infino".into()];

        report.emit(&Section {
            anchor: "comparison/vector".into(),
            title: format!(
                "Vector comparison ({} docs × dim={})",
                fmt_count(n_docs),
                corpus::dim()
            ),
            note: "All engines driven through the same `run_vector` build driver and \
                   corpus. Search tables follow below: per-engine default serving via the \
                   shared `run_search` protocol, plus a side-by-side summary. Δ is vs \
                   infino baseline."
                .into(),
            blocks: vec![
                Block {
                    subtitle: "Build — Time".into(),
                    headers: build_headers.clone(),
                    rows: build_time_rows,
                },
                Block {
                    subtitle: "Build — Throughput".into(),
                    headers: build_headers.clone(),
                    rows: build_thr_rows,
                },
                Block {
                    subtitle: "Build — Bandwidth".into(),
                    headers: build_headers.clone(),
                    rows: build_bw_rows,
                },
                Block {
                    subtitle: "Build — Peak RSS".into(),
                    headers: build_headers.clone(),
                    rows: build_peak_rows,
                },
                Block {
                    subtitle: "Build — Median RSS".into(),
                    headers: build_headers.clone(),
                    rows: build_med_rows,
                },
                Block {
                    subtitle: "Build — P90 RSS".into(),
                    headers: build_headers.clone(),
                    rows: build_p90_rows,
                },
            ],
        });

        // Search — each engine at its own shipped defaults through the
        // shared `run_search` protocol (infino's own bench driver, grid
        // off): recall measured on the shared brute-force oracle at the
        // default operating point, warm p50 at that same point. The
        // superfile comparison stays warm-only, as before.
        let qcorr = queries_correctness();
        let tcorr = ground_truth_correctness();
        let (q_cal, gt_cal): (&[Vec<f32>], &[Vec<u32>]) = if RUN_CALIBRATION_GRID {
            (queries_calibration(), ground_truth_calibration())
        } else {
            (&[], &[])
        };
        // Engine-native defaults, mirroring infino's own superfile bench:
        // the superfile reader resolves absent options to its constants.
        let (default_nprobe, default_rerank) = {
            let o = VectorSearchOptions::default();
            (
                o.nprobe.unwrap_or(VectorSearchOptions::DEFAULT_NPROBE),
                o.rerank_mult().unwrap_or(VectorSearchOptions::RERANK_MULT),
            )
        };
        let infino_rows = exec_vec::run_search(
            &mut report,
            infino_idx.reader(),
            || NoCold,
            VEC_COLUMN,
            n_docs,
            TOP_K,
            default_nprobe,
            default_rerank,
            qcorr,
            tcorr,
            q_cal,
            gt_cal,
            exec_vec::RecallFloors::superfile(),
            true,
            false,
            0,
            !RUN_CALIBRATION_GRID,
            "comparison-vector/infino",
            "comparison/superfile/vector/infino",
            format!(
                "Superfile vector — infino, default serving ({} docs × dim={})",
                fmt_count(n_docs),
                corpus::dim()
            ),
            "Identical protocol to infino's own superfile vector bench: default \
             options, recall on the shared brute-force oracle, floor-asserted.",
        );
        emit_default_comparison(
            &mut report,
            "comparison/superfile/vector",
            format!(
                "Superfile vector comparison — default serving ({} docs × dim={})",
                fmt_count(n_docs),
                corpus::dim()
            ),
            &[("infino", &infino_rows)],
            true,
            false,
        );

        let q0 = qcorr
            .first()
            .expect("superfile vector cells need at least one query");
        emit_superfile_save_load(&mut report, &infino_idx, q0, cfg, n_docs);

        report.save();

        InfinoVectorEngine::close(&mut infino_idx);
        InfinoVectorEngine::delete(infino_idx);
    }
}

pub mod sql {
    // SPDX-License-Identifier: Apache-2.0
    // SPDX-FileCopyrightText: Copyright The Infino Authors

    //! SQL comparison bench — drives infino through the same
    //! `run_sql` driver and emits a single comparison section.

    use std::collections::HashMap;

    use infino_bench_utils::corpus::{MmapTextCorpus, parallel_writers, superfile_docs};
    use infino_bench_utils::executors::sql::{ITERS, SQL_BATTERY};
    use infino_bench_utils::harness::{EngineSqlResult, InfinoSqlEngine, SqlRunConfig, run_sql};
    use infino_bench_utils::markdown::{fmt_count, fmt_time};
    use infino_bench_utils::report::{Better, Block, Report, Section, metric, text};
    use infino_bench_utils::rss;
    use infino_bench_utils::superfile::sql::sql_rows;

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

        let results: Vec<(&str, EngineSqlResult)> = vec![(
            "infino",
            run_sql::<InfinoSqlEngine>(cfg, &rows, SQL_BATTERY),
        )];

        let mut report = Report::load("comparison-sql");

        let build_map: HashMap<(&str, usize), _> = results
            .iter()
            .flat_map(|(name, res)| res.builds.iter().map(move |b| ((*name, b.writers), b)))
            .collect();
        let query_map: HashMap<(&str, &str), _> = results
            .iter()
            .flat_map(|(name, res)| res.queries.iter().map(move |q| ((*name, q.name), q)))
            .collect();

        let engines = ["infino"];
        let peers: [&str; 0] = [];

        let writer_counts: Vec<usize> = results[0].1.builds.iter().map(|b| b.writers).collect();

        let mut build_time_rows = Vec::new();
        let mut build_peak_rows = Vec::new();
        let mut build_med_rows = Vec::new();
        let mut build_p90_rows = Vec::new();

        for w in &writer_counts {
            let base = build_map.get(&("infino", *w)).expect("infino build");
            let base_secs = base.wall.as_secs_f64();
            let base_ns = base_secs * 1e9;

            let label = text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            });
            let mut time_row = vec![label];
            let mut peak_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];
            let mut med_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];
            let mut p90_row = vec![text(if *w == 1 {
                "1 writer".into()
            } else {
                format!("{w} writers")
            })];

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
                    pct(
                        b.rss.median_rss_bytes as f64,
                        base.rss.median_rss_bytes as f64,
                    ),
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

        let build_headers = vec!["Config".into(), "infino".into()];

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

        let query_headers = vec!["Query".into(), "infino".into()];

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
