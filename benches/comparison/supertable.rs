// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Supertable object-store comparison bench.
//!
//! The Infino side of every cell IS infino's own supertable bench cell —
//! `infino_bench_utils::supertable::{fts,vector,sql}::run(Phases)`, called
//! verbatim — so its protocol, tables, and report JSON are identical to
//! `cargo bench -- supertable <modality>` in the infino repo. This file
//! adds only what bench-utils cannot ship: the LanceDB peer, built from
//! the same corpus generators and seeds, ingested through the shared
//! engine-generic drivers, and searched at its own shipped defaults via
//! the shared `exec_vec` primitives (recall reported, not floor-gated).

use std::{
    env,
    time::{Duration, Instant},
};

use infino_bench_utils::corpus::{self, MmapTextCorpus};
use infino_bench_utils::executors::fts::FTS_BATTERY;
use infino_bench_utils::executors::sql::SQL_BATTERY;
use infino_bench_utils::harness::{
    FtsQuery, SqlQuery, SqlRunConfig, VectorBuildStat, VectorEngine, VectorMetric, VectorQuery,
    VectorRunConfig, VectorSearch, run_fts, run_fts_with_index, run_sql, run_sql_with_index,
    run_vector, run_vector_with_index,
};
use infino_bench_utils::ingest::supertable::{self, TEXT_COLUMN, VEC_COLUMN};
use infino_bench_utils::markdown::{fmt_count, fmt_throughput, fmt_time};
use infino_bench_utils::report::{Better, Block, Cell, Report, Section, metric, text};
use infino_bench_utils::rss;
use infino_bench_utils::superfile::sql::sql_rows;
use infino_bench_utils::supertable::{self as st_bench, Phases};
use infino_bench_utils::tiers;
use retrievalbench::{LanceS3FtsEngine, LanceS3SqlEngine, LanceS3VectorEngine, lance_peer_label};

const EMPTY_FTS_QUERIES: &[FtsQuery] = &[];
const EMPTY_VECTOR_QUERIES: &[VectorQuery<'_>] = &[];
const EMPTY_SQL_QUERIES: &[SqlQuery] = &[];
const WARM_ITERS: usize = 20;
const COLD_ITERS: usize = 5;
const TOP_K: usize = 10;
const THREAD_MODE_ENV: &str = "INFINO_BENCH_THREAD_MODE";
/// The `k` knots reported for each codec rung. A coarse codec loses the
/// tail of the neighbourhood long before it loses the top-1, so a single
/// `k` cannot say whether a rung is usable — these are also three of the
/// four knots the drain already stamps laws for (`WIDTH_LAW_KS`).
const RECALL_KS: &[usize] = &[1, 10, 100];
/// Deepest knot in [`RECALL_KS`]: one exact oracle is computed here and
/// every shallower `k` is its sorted prefix.
const RECALL_KS_DEEPEST: usize = 100;
/// Held-out queries the codec curve grades over. Sized for RESOLUTION,
/// not runtime: comparing codecs that differ by a point of recall needs a
/// quantum well under a point, and the shallowest knot (`k = 1`) has the
/// coarsest one — 1/[`CURVE_QUERIES`]. The extra cost is one exact-oracle
/// pass, which is parallel and seconds at these corpus sizes.
const CURVE_QUERIES: usize = 200;

fn lance_fts_ingest_row(n_docs: usize) -> Vec<Cell> {
    eprintln!(
        "[comparison-supertable] building LanceDB FTS-only peer on the object store over {} docs...",
        fmt_count(n_docs)
    );
    let corpus = MmapTextCorpus::generate(n_docs, 1);
    let docs = corpus.rows();
    let result = run_fts::<LanceS3FtsEngine>(TEXT_COLUMN, &docs, EMPTY_FTS_QUERIES, 10, 1, 1);
    let build = result
        .builds
        .first()
        .expect("lancedb object-store build row");
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
        "[comparison-supertable] building LanceDB vector-only peer on the object store over {} docs...",
        fmt_count(n_docs)
    );
    let prepared = supertable::prepare_corpus(supertable::Modality::Vector);
    let vectors = prepared
        .vectors()
        .expect("vector modality prepares a vector corpus");
    let cfg = VectorRunConfig {
        column: VEC_COLUMN,
        dim: corpus::dim(),
        metric: VectorMetric::Cosine,
        k: TOP_K,
        iters: 1,
        parallel: 1,
    };
    let vslice = &vectors.as_slice()[..n_docs * corpus::dim()];
    let result = run_vector::<LanceS3VectorEngine>(cfg, vslice, EMPTY_VECTOR_QUERIES);
    let build = result
        .builds
        .first()
        .expect("lancedb object-store vector build row");
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
        "[comparison-supertable] building LanceDB SQL peer on the object store over {} docs...",
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
    let build = result
        .builds
        .first()
        .expect("lancedb object-store sql build row");
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

/// The peer ingest table. Infino's ingest table (isolated shape
/// subprocesses) is emitted by infino's own bench cells running in this
/// same invocation; this section adds only the LanceDB rows, once per
/// process.
pub fn run() {
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

    let n_docs = supertable::n_docs();
    let rows = vec![
        lance_fts_ingest_row(n_docs),
        lance_vector_ingest_row(n_docs),
        lance_sql_ingest_row(n_docs),
    ];

    let mut report = Report::load("comparison-supertable");
    report.emit(&Section {
        anchor: "comparison/supertable/ingest".into(),
        title: format!(
            "Supertable comparison — {} ingest, object store ({} docs × dim={})",
            lance_peer_label(),
            fmt_count(n_docs),
            corpus::dim()
        ),
        note: "LanceDB peer ingest rows, driven by the shared `run_fts`/`run_vector`/`run_sql` \
               drivers with object-store-configured adapters (INFINO_BENCH_STORE: s3 or azure). \
               The Infino ingest table (isolated shape subprocesses) is emitted by infino's own \
               supertable bench cells running in this same invocation."
            .into(),
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

pub mod fts {
    use super::*;
    use infino_bench_utils::executors::fts as exec_fts;
    use infino_bench_utils::executors::fts::FtsRead;
    use retrievalbench::lance::fts::LanceFtsColdGuard;

    pub fn run(build: bool, warm: bool, cold: bool) {
        if let Err(reason) = tiers::supertable_backend_check() {
            eprintln!("[comparison-supertable-fts] skipped: {reason}");
            return;
        }
        // Infino: infino's own supertable FTS bench cell, verbatim.
        st_bench::fts::run(Phases { build, warm, cold });
        if build {
            super::run();
        }
        if !(warm || cold) {
            return;
        }

        // Peer: LanceDB dataset from the same corpus generator; the
        // batteries below run through the SAME `exec_fts` protocol
        // machinery infino's cell uses (search + fetch phases, count,
        // cold on fresh opens) — only the `FtsRead` impl is lance's.
        let n_docs = supertable::n_docs();
        let corpus = MmapTextCorpus::generate(n_docs, 1);
        let docs = corpus.rows();
        let (_build, lance_index) = run_fts_with_index::<LanceS3FtsEngine>(
            TEXT_COLUMN,
            &docs,
            EMPTY_FTS_QUERIES,
            TOP_K,
            1,
            1,
        );

        let mut report = Report::load("comparison-supertable-fts");
        let warm_stats = warm.then(|| {
            // Prewarm every battery shape once — the mirror of the
            // in-tree cell's consumer prewarm before its warm rows.
            for q in FTS_BATTERY {
                let query = q.terms.join(" ");
                let _ = lance_index.bm25_rows(
                    TEXT_COLUMN,
                    &query,
                    TOP_K,
                    exec_fts::to_infino_mode(q.mode),
                );
            }
            exec_fts::measure_warm(
                &lance_index,
                FTS_BATTERY,
                TEXT_COLUMN,
                TOP_K,
                WARM_ITERS,
                "comparison-supertable-fts/lancedb",
            )
        });
        let counts = warm.then(|| {
            exec_fts::measure_count(
                &lance_index,
                FTS_BATTERY,
                TEXT_COLUMN,
                WARM_ITERS,
                "comparison-supertable-fts/lancedb",
            )
        });
        let cold_stats = cold.then(|| {
            exec_fts::measure_cold(
                || LanceFtsColdGuard::open(&lance_index),
                FTS_BATTERY,
                TEXT_COLUMN,
                TOP_K,
                COLD_ITERS,
                true,
                "comparison-supertable-fts/lancedb",
            )
        });
        exec_fts::emit_search(
            &mut report,
            "comparison/supertable/fts/lancedb",
            format!(
                "Supertable FTS — {}, queries + cost ({} docs)",
                lance_peer_label(),
                fmt_count(n_docs)
            ),
            "Peer battery through the same `exec_fts` protocol as infino's own cell: \
             search phase (id + score) and fetch phase (+ top-k text), cold on fresh \
             table opens per iteration. Infino's tables are emitted by its own bench \
             cell in this same invocation.",
            warm_stats.as_deref(),
            cold_stats.as_ref(),
            None,
        );
        if let Some(counts) = &counts {
            exec_fts::emit_count(
                &mut report,
                "comparison/supertable/fts/lancedb/count",
                format!(
                    "Supertable FTS — {}, count ({} docs)",
                    lance_peer_label(),
                    fmt_count(n_docs)
                ),
                "Count via normal SQL: COUNT(*) aggregated in the engine pipeline over \
                 the FTS-matched lance provider — only the scalar crosses, matching \
                 infino's count path returning a count, not ids.",
                counts,
            );
        }
        report.save();
    }
}

pub mod vector {
    use super::*;
    use crate::superfile::vector::peer_default_rows;
    use infino_bench_utils::executors::vector as exec_vec;
    use retrievalbench::lance::vector::LanceVecColdGuard;
    // Compressed-flat peers, kept separate from table-level search.
    use infino_bench_utils::cpu;
    use infino_bench_utils::rss::PeakSampler;
    use retrievalbench::{
        Sq4FlatVectorEngine, Sq4ResidualFlatVectorEngine, Turbovec2VectorEngine,
        Turbovec4VectorEngine,
    };

    const INPLACE_SINGLE: usize = 1;
    const INPLACE_BATCH: usize = 100;
    const NS_PER_SEC: f64 = 1e9;
    type CodecKRow = (usize, f32, f64, f64, usize);

    /// One `(k, k-recall, p50_ns, p95_ns, resident_bytes)` row per knot in
    /// [`RECALL_KS`], all from a single exact oracle computed at
    /// [`RECALL_KS_DEEPEST`] and truncated per knot (`recall_at_k`
    /// divides by the truth row's length, so the truth must carry
    /// exactly `k` entries). Search runs at the engine's own defaults.
    fn per_k_rows<R: exec_vec::VectorRead>(
        reader: &R,
        queries: &[Vec<f32>],
        gt_deep: &[Vec<u32>],
        resident_bytes: usize,
    ) -> Vec<CodecKRow> {
        RECALL_KS
            .iter()
            .map(|&k| {
                let truths: Vec<Vec<u32>> = gt_deep
                    .iter()
                    .map(|t| t[..k.min(t.len())].to_vec())
                    .collect();
                let mut recall_sum = 0.0_f32;
                let mut latencies = Vec::with_capacity(queries.len());
                for (query, truth) in queries.iter().zip(&truths) {
                    let started = Instant::now();
                    let hits = reader.topk_global(
                        VEC_COLUMN,
                        query,
                        k,
                        exec_vec::ENGINE_DEFAULT,
                        exec_vec::ENGINE_DEFAULT,
                    );
                    latencies.push(started.elapsed());
                    recall_sum += corpus::recall_at_k(&hits, truth);
                }
                latencies.sort_unstable();
                let percentile = |pct: usize| {
                    let rank = (pct * latencies.len()).div_ceil(100);
                    latencies[rank.saturating_sub(1).min(latencies.len() - 1)]
                };
                (
                    k,
                    recall_sum / queries.len() as f32,
                    percentile(50).as_secs_f64() * 1e9,
                    percentile(95).as_secs_f64() * 1e9,
                    resident_bytes,
                )
            })
            .collect()
    }

    /// Native add / remove / save / load on an already-built peer index.
    /// Infino superfile has no equivalent in-place path.
    fn timed_op(f: impl FnOnce()) -> (Duration, u64) {
        let sampler = PeakSampler::start_default();
        let ((), wall, _) = cpu::timed(f);
        (wall, sampler.stop_stats().peak_rss_bytes)
    }

    fn codec_lifecycle<E: VectorEngine>(
        report: &mut Report,
        index: &mut E::Index,
        extra_vectors: &[f32],
        dim: usize,
        query: &[f32],
        k: usize,
        n_docs: usize,
    ) {
        assert!(
            extra_vectors.len() >= INPLACE_BATCH * dim,
            "codec lifecycle needs at least {INPLACE_BATCH} held-out rows"
        );
        let capabilities = E::capabilities();
        assert!(
            capabilities.vector_insert
                && capabilities.vector_remove
                && capabilities.vector_save_load,
            "{} must support native add/remove/save/load",
            E::name()
        );
        let search = VectorSearch {
            nprobe: exec_vec::ENGINE_DEFAULT,
            rerank_mult: exec_vec::ENGINE_DEFAULT,
        };
        let extra_1 = &extra_vectors[..INPLACE_SINGLE * dim];
        let extra_100 = &extra_vectors[..INPLACE_BATCH * dim];

        let (save_wall, save_rss) = timed_op(|| {
            let bytes = E::save(index).expect("codec save");
            std::hint::black_box(bytes.len());
        });
        let saved = E::save(index).expect("codec save bytes");
        let (load_wall, load_rss) = timed_op(|| {
            let loaded =
                E::load(VEC_COLUMN, dim, VectorMetric::Cosine, &saved).expect("codec load");
            std::hint::black_box(loaded);
        });
        let loaded =
            E::load(VEC_COLUMN, dim, VectorMetric::Cosine, &saved).expect("load for first search");
        let (first_wall, first_rss) = timed_op(|| {
            let hits = E::read(&loaded, query, k, search);
            std::hint::black_box(hits);
        });

        let mut next_id = n_docs as u64;
        let (ins1_wall, ins1_rss) = timed_op(|| {
            assert!(E::insert(index, extra_1, next_id));
        });
        next_id += INPLACE_SINGLE as u64;
        let (ins100_wall, ins100_rss) = timed_op(|| {
            assert!(E::insert(index, extra_100, next_id));
        });
        next_id += INPLACE_BATCH as u64;

        let last = next_id - 1;
        let (rem1_wall, rem1_rss) = timed_op(|| {
            assert!(E::remove(index, &[last]));
        });
        next_id -= 1;
        let rem100: Vec<u64> = (next_id - INPLACE_BATCH as u64..next_id).collect();
        let (rem100_wall, rem100_rss) = timed_op(|| {
            assert!(E::remove(index, &rem100));
        });

        // mutate → save → reopen → first query
        assert!(E::insert(index, extra_1, n_docs as u64));
        let mutated = E::save(index).expect("codec save after insert");
        let (reopen_wall, reopen_rss) = timed_op(|| {
            let reopened = E::load(VEC_COLUMN, dim, VectorMetric::Cosine, &mutated)
                .expect("codec load after mutate");
            let hits = E::read(&reopened, query, k, search);
            std::hint::black_box(hits);
        });

        fn row(label: &str, wall: Duration, rss: u64) -> Vec<Cell> {
            let ns = wall.as_secs_f64() * NS_PER_SEC;
            vec![
                text(label),
                metric(ns, fmt_time(ns), Better::Lower),
                metric(rss as f64, rss::fmt_bytes(rss), Better::Lower),
            ]
        }

        report.emit(&Section {
            anchor: format!("comparison/supertable/vector/codec-lifecycle/{}", E::name()),
            title: format!(
                "Codec lifecycle — {} add/remove/save/load ({} docs × dim={})",
                E::name(),
                fmt_count(n_docs),
                dim
            ),
            note: "Native in-place add/remove and native save/load on the same index the \
                   recall curve just built. Infino superfile has no equivalent mutable \
                   path — table `append`/`delete` is a separate cell."
                .into(),
            blocks: vec![Block {
                subtitle: E::name().into(),
                headers: vec!["Op".into(), "Wall".into(), "Peak RSS".into()],
                rows: vec![
                    row("save", save_wall, save_rss),
                    row("load", load_wall, load_rss),
                    row("load → first search", first_wall, first_rss),
                    row("add n=1", ins1_wall, ins1_rss),
                    row("add n=100", ins100_wall, ins100_rss),
                    row("remove n=1", rem1_wall, rem1_rss),
                    row("remove n=100", rem100_wall, rem100_rss),
                    row(
                        "mutate → save → reopen → first query",
                        reopen_wall,
                        reopen_rss,
                    ),
                ],
            }],
        });
    }

    /// The codec comparison: our terminal-ranking rungs beside the peer's,
    /// on the corpus infino ingests in this same run.
    ///
    /// Runs BEFORE infino's own lifecycle cell rather than after it. The
    /// measurement depends only on the corpus, never on the lifecycle, and
    /// ordering it first means a tripwire in some later lifecycle phase
    /// (filtered-recall floors, cold-read ceilings) can no longer take the
    /// comparison rows down with it — which is exactly what cost the
    /// glove-200 peer rows on the first run.
    pub(crate) fn codec_curve() {
        let thread_mode =
            env::var(THREAD_MODE_ENV).unwrap_or_else(|_| "unspecified-threads".into());
        let n_docs = supertable::n_docs();
        let prepared = supertable::prepare_corpus(supertable::Modality::Vector);
        let vectors = prepared
            .vectors()
            .expect("vector modality prepares a vector corpus");
        let vslice = &vectors.as_slice()[..n_docs * corpus::dim()];
        let cfg = VectorRunConfig {
            column: VEC_COLUMN,
            dim: corpus::dim(),
            metric: VectorMetric::Cosine,
            k: TOP_K,
            iters: WARM_ITERS,
            parallel: 1,
        };
        // Same generator, seed and slice infino's own cell uses, so both
        // sides answer the identical query set.
        //
        // Deliberately NOT the correctness battery's 20 queries: this
        // curve exists to separate codecs that differ by a point or two
        // of recall, and 20 queries cannot. At k=1 each query is worth
        // 0.05 recall, so a two-query difference reads as 10 points and
        // invites exactly the false attribution it should settle. At
        // [`CURVE_QUERIES`] the k=1 quantum is 0.005 and the k=10 quantum
        // 0.0005, which is finer than the differences being compared.
        let q_corr = if vectors.n_docs() > n_docs {
            corpus::bench_queries(
                vslice,
                n_docs,
                CURVE_QUERIES,
                exec_vec::QUERY_CORRECTNESS_SEED,
                true,
                exec_vec::QUERY_SIGMA,
            )
        } else {
            // A corpus may end exactly at the advertised rung (DBpedia has
            // exactly 1M rows). Deterministic perturbed members remain
            // out-of-index queries without relabelling a 999.9K run as 1M.
            corpus::generate_realistic_queries(
                vslice,
                n_docs,
                CURVE_QUERIES,
                exec_vec::QUERY_CORRECTNESS_SEED,
                true,
                exec_vec::QUERY_SIGMA,
            )
        };
        let tail = &vectors.as_slice()[n_docs * corpus::dim()..];
        let generated_extra;
        let extra_vectors = if tail.len() >= INPLACE_BATCH * corpus::dim() {
            tail
        } else {
            generated_extra = q_corr
                .iter()
                .take(INPLACE_BATCH)
                .flatten()
                .copied()
                .collect::<Vec<_>>();
            generated_extra.as_slice()
        };
        let q0 = q_corr
            .first()
            .expect("codec curve needs at least one query");
        let mut report = Report::load("comparison-supertable-vector-codec");
        // Recall at @1, @10 AND @100 differs materially because a coarse
        // codec loses the tail of the neighbourhood long before it loses
        // the top-1. Measured over the same `vslice` Infino ingested, the
        // SAME held-out queries, and the SAME exact oracle, in this one
        // process, so these rows sit directly beside the infino arm's.
        //
        // `recall_at_k` divides by the truth row's length, so a per-`k`
        // row needs the oracle truncated to that `k`; one exact oracle at
        // the deepest `k` supplies all three by prefix (it is sorted).
        //
        // No cold column: a compressed flat index is resident by
        // construction, so "cold" would time a file load, not the
        // object-store fetch Infino's cold column reports. These rows carry
        // nq=1 warm p50, k-recall, and serialized/resident bytes.
        let gt_deep = retrievalbench::cohere::ground_truth(
            corpus::corpus_source(),
            n_docs,
            q_corr.len(),
            RECALL_KS_DEEPEST,
        )
        .unwrap_or_else(|| corpus::ground_truth(vslice, n_docs, &q_corr, RECALL_KS_DEEPEST));
        let mut curve_rows = Vec::new();
        // Build stats the shared driver measures for EVERY engine
        // (wall + on-CPU seconds + peak RSS). Previously bound to
        // throwaway names here, so the codec table published search
        // rows while the already-measured build costs died in a local.
        let mut build_rows: Vec<(&'static str, Vec<VectorBuildStat>)> = Vec::new();
        let (tv4_res, mut tv4) =
            run_vector_with_index::<Turbovec4VectorEngine>(cfg, vslice, EMPTY_VECTOR_QUERIES);
        build_rows.push(("turbovec-4bit", tv4_res.builds));
        let tv4_rows = per_k_rows(&tv4, &q_corr, &gt_deep, tv4.index_bytes());
        let (tv2_res, mut tv2) =
            run_vector_with_index::<Turbovec2VectorEngine>(cfg, vslice, EMPTY_VECTOR_QUERIES);
        build_rows.push(("turbovec-2bit", tv2_res.builds));
        #[allow(unused_mut)] // mutated only when the `faiss` feature is on
        let mut curve_engines: Vec<(&str, Vec<CodecKRow>)> = vec![
            ("turbovec-4bit", tv4_rows),
            (
                "turbovec-2bit",
                per_k_rows(&tv2, &q_corr, &gt_deep, tv2.index_bytes()),
            ),
            // Our own codec ranking terminally, which is the only
            // configuration comparable to a compressed flat index:
            // recall bounded by quantization error alone, and no
            // adjacency in the byte count.
            ("infino-sq4-flat", {
                let (res, idx) =
                    run_vector_with_index::<Sq4FlatVectorEngine>(cfg, vslice, EMPTY_VECTOR_QUERIES);
                build_rows.push(("infino-sq4-flat", res.builds));
                per_k_rows(&idx, &q_corr, &gt_deep, idx.resident_bytes())
            }),
            ("infino-sq4res-flat", {
                let (res, idx) = run_vector_with_index::<Sq4ResidualFlatVectorEngine>(
                    cfg,
                    vslice,
                    EMPTY_VECTOR_QUERIES,
                );
                build_rows.push(("infino-sq4res-flat", res.builds));
                per_k_rows(&idx, &q_corr, &gt_deep, idx.resident_bytes())
            }),
        ];
        #[cfg(feature = "faiss")]
        {
            use retrievalbench::{FaissPqFastScanVectorEngine, FaissPqVectorEngine};
            let (res, mut idx) = run_vector_with_index::<FaissPqFastScanVectorEngine>(
                cfg,
                vslice,
                EMPTY_VECTOR_QUERIES,
            );
            build_rows.push(("faiss-pq-fastscan", res.builds));
            curve_engines.push((
                "faiss-pq-fastscan",
                per_k_rows(&idx, &q_corr, &gt_deep, idx.serialized_bytes()),
            ));
            if thread_mode == "single-thread" {
                codec_lifecycle::<FaissPqFastScanVectorEngine>(
                    &mut report,
                    &mut idx,
                    extra_vectors,
                    corpus::dim(),
                    q0,
                    TOP_K,
                    n_docs,
                );
            }
            FaissPqFastScanVectorEngine::close(&mut idx);
            FaissPqFastScanVectorEngine::delete(idx);
            let (res, mut idx) =
                run_vector_with_index::<FaissPqVectorEngine>(cfg, vslice, EMPTY_VECTOR_QUERIES);
            build_rows.push(("faiss-pq", res.builds));
            curve_engines.push((
                "faiss-pq",
                per_k_rows(&idx, &q_corr, &gt_deep, idx.serialized_bytes()),
            ));
            if thread_mode == "single-thread" {
                codec_lifecycle::<FaissPqVectorEngine>(
                    &mut report,
                    &mut idx,
                    extra_vectors,
                    corpus::dim(),
                    q0,
                    TOP_K,
                    n_docs,
                );
            }
            FaissPqVectorEngine::close(&mut idx);
            FaissPqVectorEngine::delete(idx);
        }
        for (name, engine_rows) in curve_engines {
            for (k, recall, p50_ns, p95_ns, bytes) in engine_rows {
                eprintln!(
                    "[codec-curve] {name} recall@{k} = {recall:.3}  p50 = {:.3} ms  \
                         resident = {}  ({} B/vec)",
                    p50_ns / 1e6,
                    rss::fmt_bytes(bytes as u64),
                    bytes / n_docs.max(1),
                );
                curve_rows.push(vec![
                    text(format!("{name} @{k}")),
                    metric(recall as f64, format!("{recall:.3}"), Better::Higher),
                    metric(p50_ns, fmt_time(p50_ns), Better::Lower),
                    metric(p95_ns, fmt_time(p95_ns), Better::Lower),
                    metric(bytes as f64, rss::fmt_bytes(bytes as u64), Better::Lower),
                    metric(
                        (bytes / n_docs.max(1)) as f64,
                        format!("{}", bytes / n_docs.max(1)),
                        Better::Lower,
                    ),
                ]);
            }
        }
        let mut codec_build_rows = Vec::new();
        for (name, builds) in &build_rows {
            for b in builds {
                let wall_ns = b.wall.as_nanos() as f64;
                eprintln!(
                    "[codec-build] {name} writers={} wall = {}  cpu = {}  peak rss = {}",
                    b.writers,
                    fmt_time(wall_ns),
                    b.cpu_s
                        .map(|c| format!("{c:.2}s"))
                        .unwrap_or_else(|| "not sampled".into()),
                    rss::fmt_bytes(b.rss.peak_rss_bytes),
                );
                codec_build_rows.push(vec![
                    text(format!("{name} ({} writer)", b.writers)),
                    metric(wall_ns, fmt_time(wall_ns), Better::Lower),
                    metric(
                        b.cpu_s.unwrap_or(0.0),
                        b.cpu_s
                            .map(|c| format!("{c:.2} s"))
                            .unwrap_or_else(|| "—".into()),
                        Better::Lower,
                    ),
                    metric(
                        b.rss.peak_rss_bytes as f64,
                        rss::fmt_bytes(b.rss.peak_rss_bytes),
                        Better::Lower,
                    ),
                ]);
            }
        }
        report.emit(&Section {
            anchor: "comparison/supertable/vector/codec-build".into(),
            title: format!(
                "Compressed flat codec build — {} ({} docs × dim={})",
                thread_mode,
                fmt_count(n_docs),
                corpus::dim()
            ),
            note: "Build cost of each compressed flat index over the same corpus \
                   slice, measured by the shared `run_vector` driver (wall, \
                   all-thread on-CPU seconds, peak RSS). The infino table arm's \
                   build is the ingest cell's job and reports there, not here."
                .into(),
            blocks: vec![Block {
                subtitle: format!("Build — {thread_mode}"),
                headers: vec![
                    "Engine".into(),
                    "Build wall".into(),
                    "Build CPU".into(),
                    "Peak RSS".into(),
                ],
                rows: codec_build_rows,
            }],
        });
        report.emit(&Section {
            anchor: "comparison/supertable/vector/codec-curve".into(),
            title: format!(
                "Compressed flat codec curve — {} ({} docs × dim={})",
                thread_mode,
                fmt_count(n_docs),
                corpus::dim()
            ),
            note: "k-recall (|top-k ∩ exact top-k| / k) on the same exact oracle and \
                       the same held-out queries as the infino rows in this run — the \
                       first neighbourhood-recall measurement for TurboQuant, whose own \
                       published figure is 1-recall@k (does the true #1 appear in the \
                       top-k), which saturates by k=4. Latency is true single-query \
                       (nq = 1), not their batch-amortized number. Resident is the \
                       exact serialized index."
                .into(),
            blocks: vec![Block {
                subtitle: format!("Rung × k — {thread_mode}"),
                headers: vec![
                    "Engine / k".into(),
                    "k-recall".into(),
                    "warm p50 (nq=1)".into(),
                    "warm p95 (nq=1)".into(),
                    "resident".into(),
                    "B/vec".into(),
                ],
                rows: curve_rows,
            }],
        });
        if thread_mode == "single-thread" {
            codec_lifecycle::<Turbovec4VectorEngine>(
                &mut report,
                &mut tv4,
                extra_vectors,
                corpus::dim(),
                q0,
                TOP_K,
                n_docs,
            );
            codec_lifecycle::<Turbovec2VectorEngine>(
                &mut report,
                &mut tv2,
                extra_vectors,
                corpus::dim(),
                q0,
                TOP_K,
                n_docs,
            );
        }
        Turbovec4VectorEngine::close(&mut tv4);
        Turbovec4VectorEngine::delete(tv4);
        Turbovec2VectorEngine::close(&mut tv2);
        Turbovec2VectorEngine::delete(tv2);
        report.save();
    }

    pub fn run(build: bool, warm: bool, cold: bool) {
        if let Err(reason) = tiers::supertable_backend_check() {
            eprintln!("[comparison-supertable-vector] skipped: {reason}");
            return;
        }
        // Infino: infino's own supertable vector bench cell, verbatim —
        // ingest shapes, default (law-served) search, recall floors, and
        // its report tables all come from bench-utils unchanged.
        st_bench::vector::run(Phases { build, warm, cold });
        if build {
            super::run();
        }
        if !(warm || cold) {
            return;
        }

        // Peer: LanceDB over the SAME corpus infino just ingested —
        // `prepare_corpus` is the selector infino's own cell uses, so a
        // real dataset (annb / hf / parquet) and the synthetic generator
        // both land here identically and the two engines can never index
        // different bytes.
        let n_docs = supertable::n_docs();
        let prepared = supertable::prepare_corpus(supertable::Modality::Vector);
        let vectors = prepared
            .vectors()
            .expect("vector modality prepares a vector corpus");
        let cfg = VectorRunConfig {
            column: VEC_COLUMN,
            dim: corpus::dim(),
            metric: VectorMetric::Cosine,
            k: TOP_K,
            iters: WARM_ITERS,
            parallel: 1,
        };
        // `prepare_corpus` materializes base + one delta commit; index
        // and grade the ingested prefix only, as infino's cell does.
        let vslice = &vectors.as_slice()[..n_docs * corpus::dim()];
        let (_lance_build, lance_index) =
            run_vector_with_index::<LanceS3VectorEngine>(cfg, vslice, EMPTY_VECTOR_QUERIES);

        // Same held-out query protocol as infino's own bench cell:
        // `bench_queries` dispatches by corpus — the dataset's own test
        // set for annb, rows past the ingested prefix for parquet/hf,
        // perturbed corpus members for synthetic.
        let q_corr = corpus::bench_queries(
            vslice,
            n_docs,
            exec_vec::N_CORRECTNESS_QUERIES,
            exec_vec::QUERY_CORRECTNESS_SEED,
            true,
            exec_vec::QUERY_SIGMA,
        );
        let gt_corr = retrievalbench::cohere::ground_truth(
            corpus::corpus_source(),
            n_docs,
            q_corr.len(),
            TOP_K,
        )
        .unwrap_or_else(|| corpus::ground_truth(vslice, n_docs, &q_corr, TOP_K));

        let mut report = Report::load("comparison-supertable-vector");
        let lance_rows = peer_default_rows(
            &lance_index,
            || LanceVecColdGuard::open(&lance_index),
            VEC_COLUMN,
            &q_corr,
            &gt_corr,
            TOP_K,
            warm,
            cold,
            COLD_ITERS,
            "comparison-supertable-vector/lancedb",
        );
        exec_vec::emit_recall_table(
            &mut report,
            "comparison/supertable/vector/lancedb",
            format!(
                "Supertable vector — {}, default serving ({} docs × dim={})",
                lance_peer_label(),
                fmt_count(n_docs),
                corpus::dim()
            ),
            "Peer default row through the same `exec_vec` primitives infino's own bench \
             uses; recall at LanceDB's own shipped search defaults is reported, not \
             floor-gated. cold = fresh table open per iteration. Infino's search table \
             is emitted by its own bench cell in this same invocation.",
            &lance_rows,
            warm,
            cold,
        );

        report.save();
    }
}

pub mod sql {
    use super::*;
    use infino_bench_utils::executors::ColdTiming;
    use infino_bench_utils::executors::sql as exec_sql;
    use retrievalbench::lance::sql::LanceSqlColdGuard;
    use std::collections::HashMap;

    pub fn run(build: bool, warm: bool, cold: bool) {
        if let Err(reason) = tiers::supertable_backend_check() {
            eprintln!("[comparison-supertable-sql] skipped: {reason}");
            return;
        }
        // Infino: infino's own supertable SQL bench cell, verbatim.
        st_bench::sql::run(Phases { build, warm, cold });
        if build {
            super::run();
        }
        if !(warm || cold) {
            return;
        }

        // Peer: LanceDB dataset from the same scalar rows. Warm runs the
        // shared `run_sql` driver battery; cold runs the shared
        // `exec_sql::measure_cold` (fresh open per iteration, GETs
        // metered) with the lance `SqlRead` guard.
        let n_docs = supertable::n_docs();
        let corpus = MmapTextCorpus::generate(n_docs, 1);
        let corpus_rows = corpus.rows();
        let rows = sql_rows(&corpus_rows);
        let cfg = SqlRunConfig {
            iters: WARM_ITERS,
            parallel: 1,
        };
        let (lance_warm, lance_index) =
            run_sql_with_index::<LanceS3SqlEngine>(cfg, &rows, SQL_BATTERY);

        let mut report = Report::load("comparison-supertable-sql");
        let warm_rows: Option<Vec<(&'static str, Duration)>> =
            warm.then(|| lance_warm.queries.iter().map(|q| (q.name, q.p50)).collect());
        let cold_map: Option<HashMap<&'static str, ColdTiming>> = cold.then(|| {
            let battery: Vec<(&'static str, &str)> =
                SQL_BATTERY.iter().map(|q| (q.name, q.sql)).collect();
            exec_sql::measure_cold(
                || LanceSqlColdGuard::open(&lance_index),
                &battery,
                COLD_ITERS,
                "comparison-supertable-sql/lancedb",
            )
        });
        emit_peer_sql(
            &mut report,
            "comparison/supertable/sql/lancedb",
            format!(
                "Supertable SQL — {} queries ({} rows)",
                lance_peer_label(),
                fmt_count(n_docs)
            ),
            "Warm = shared `run_sql` driver battery on a warmed table handle; cold = \
             the shared `exec_sql::measure_cold` (fresh connection + provider per \
             iteration, first real lance scan timed, object-store GETs metered where \
             instrumented). Infino's tables are emitted by its own bench cell in this \
             same invocation.",
            warm_rows.as_deref(),
            cold_map.as_ref(),
        );
        report.save();
    }

    /// Rendering-only: one row per battery shape from measurements the
    /// shared drivers produced above. Never measures.
    fn emit_peer_sql(
        report: &mut Report,
        anchor: &str,
        title: String,
        note: &str,
        warm: Option<&[(&'static str, Duration)]>,
        cold: Option<&HashMap<&'static str, ColdTiming>>,
    ) {
        const NS_PER_SEC: f64 = 1e9;
        let mut headers = vec!["Query".to_string()];
        if warm.is_some() {
            headers.push("warm p50".into());
        }
        if cold.is_some() {
            headers.push("cold open".into());
            headers.push("cold 1st query".into());
            headers.push("cold GETs".into());
        }
        let names: Vec<&'static str> = SQL_BATTERY.iter().map(|q| q.name).collect();
        let rows = names
            .iter()
            .map(|name| {
                let mut row = vec![text((*name).to_string())];
                if let Some(warm) = warm {
                    match warm.iter().find(|(n, _)| n == name) {
                        Some((_, d)) => {
                            let ns = d.as_secs_f64() * NS_PER_SEC;
                            row.push(metric(ns, fmt_time(ns), Better::Lower));
                        }
                        None => row.push(text(String::from("—"))),
                    }
                }
                if let Some(cold) = cold {
                    match cold.get(name) {
                        Some(c) => {
                            let open_ns = c.open.as_secs_f64() * NS_PER_SEC;
                            let search_ns = c.search.as_secs_f64() * NS_PER_SEC;
                            row.push(metric(open_ns, fmt_time(open_ns), Better::Lower));
                            row.push(metric(search_ns, fmt_time(search_ns), Better::Lower));
                            row.push(metric(
                                c.search_get_count as f64,
                                format!("{}", c.search_get_count),
                                Better::Lower,
                            ));
                        }
                        None => {
                            row.push(text(String::from("—")));
                            row.push(text(String::from("—")));
                            row.push(text(String::from("—")));
                        }
                    }
                }
                row
            })
            .collect();
        report.emit(&Section {
            anchor: anchor.into(),
            title,
            note: note.into(),
            blocks: vec![Block {
                subtitle: format!("{} — SQL battery", lance_peer_label()),
                headers,
                rows,
            }],
        });
    }
}
