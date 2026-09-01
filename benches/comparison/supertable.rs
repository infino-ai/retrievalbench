// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Supertable object-store comparison bench.
//!
//! The Infino side of every cell IS infino's own supertable bench cell —
//! `infino_bench_utils::supertable::{fts,vector,sql}::run(Phases)`, called
//! verbatim — so its protocol, tables, and report JSON are identical to
//! `cargo bench -- supertable <modality>` in the infino repo. This file
//! adds the comparison-only cells — the codec table and the write
//! cells — on top of those.

use std::{env, sync::Arc, time::Duration};

use infino::config::{VectorSearchMode, global as engine_config};
use infino_bench_utils::corpus;
use infino_bench_utils::harness::{
    VectorBuildStat, VectorEngine, VectorMetric, VectorQuery, VectorRunConfig, VectorSearch,
    run_vector_with_index,
};
use infino_bench_utils::ingest::supertable::{self, VEC_COLUMN};
use infino_bench_utils::markdown::{fmt_count, fmt_time};
use infino_bench_utils::report::{Better, Block, Cell, Report, Section, metric, text};
use infino_bench_utils::rss;
use infino_bench_utils::supertable::{self as st_bench, Phases};
use infino_bench_utils::tiers;

const EMPTY_VECTOR_QUERIES: &[VectorQuery<'_>] = &[];
const WARM_ITERS: usize = 20;
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
/// coarsest one — 1/[`CURVE_QUERIES`]. At 1000 the standard error on a
/// mean recall is ~±0.004, so a two-point codec gap stands on a single
/// run instead of leaning on repeats. The extra cost is one exact-oracle
/// pass, which is parallel and minutes at the largest corpus size.
const CURVE_QUERIES: usize = 1000;

pub mod fts {
    use super::*;

    pub fn run(build: bool, warm: bool, cold: bool) {
        if let Err(reason) = tiers::supertable_backend_check() {
            eprintln!("[comparison-supertable-fts] skipped: {reason}");
            return;
        }
        // Infino: infino's own supertable FTS bench cell, verbatim.
        st_bench::fts::run(Phases { build, warm, cold });
    }
}

pub mod vector {
    use super::*;
    use infino_bench_utils::executors::vector as exec_vec;
    // Compressed-flat peers, kept separate from table-level search.
    use infino_bench_utils::cpu;
    use infino_bench_utils::rss::PeakSampler;
    use retrievalbench::{Turbovec2VectorEngine, Turbovec4VectorEngine};

    const INPLACE_SINGLE: usize = 1;
    const INPLACE_BATCH: usize = 100;
    /// Discarded mutations before any mutation cell is timed. The first
    /// insert or remove on a freshly built index pays one-time costs —
    /// invalidating and rebuilding the engine's packed/search-ready caches,
    /// first touch of grown buffers — that are not part of the steady-state
    /// per-op cost. Without this, `add n=1` timed slower than `add n=100`
    /// on three of four engines, which no per-op cost can explain.
    const MUTATION_WARMUP: usize = 1;
    /// Timed repetitions per mutation cell. The reported figure is the
    /// median, so one outlier sample cannot set the published number.
    const MUTATION_SAMPLES: usize = 5;
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
        exec_vec::per_k_sweep(queries, gt_deep, RECALL_KS, |query, k| {
            reader.topk_global(
                VEC_COLUMN,
                query,
                k,
                exec_vec::ENGINE_DEFAULT,
                exec_vec::ENGINE_DEFAULT,
            )
        })
        .into_iter()
        .map(|cell| {
            (
                cell.k,
                cell.recall,
                cell.p50_ns,
                cell.p95_ns,
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

    /// Median sample by wall time, so a single slow run cannot set the
    /// published figure.
    fn median_op(mut samples: Vec<(Duration, u64)>) -> (Duration, u64) {
        assert!(!samples.is_empty(), "median of no samples");
        samples.sort_by_key(|(wall, _)| *wall);
        samples[samples.len() / 2]
    }

    /// Warm up, then time [`MUTATION_SAMPLES`] inserts of `count` rows and
    /// return the median. Every repetition re-inserts the same held-out
    /// rows under fresh ids: only the row count bears on the timing.
    fn timed_inserts<E: VectorEngine>(
        index: &mut E::Index,
        rows: &[f32],
        count: usize,
        next_id: &mut u64,
    ) -> (Duration, u64) {
        for _ in 0..MUTATION_WARMUP {
            assert!(E::insert(index, rows, *next_id));
            *next_id += count as u64;
        }
        let mut samples = Vec::with_capacity(MUTATION_SAMPLES);
        for _ in 0..MUTATION_SAMPLES {
            let id = *next_id;
            samples.push(timed_op(|| assert!(E::insert(index, rows, id))));
            *next_id += count as u64;
        }
        median_op(samples)
    }

    /// The remove counterpart of [`timed_inserts`], taking the highest
    /// `count` live ids each time so the rows just inserted are the ones
    /// removed and the index returns to its original length.
    fn timed_removes<E: VectorEngine>(
        index: &mut E::Index,
        count: usize,
        next_id: &mut u64,
    ) -> (Duration, u64) {
        for _ in 0..MUTATION_WARMUP {
            let ids: Vec<u64> = (*next_id - count as u64..*next_id).collect();
            assert!(E::remove(index, &ids));
            *next_id -= count as u64;
        }
        let mut samples = Vec::with_capacity(MUTATION_SAMPLES);
        for _ in 0..MUTATION_SAMPLES {
            let ids: Vec<u64> = (*next_id - count as u64..*next_id).collect();
            samples.push(timed_op(|| assert!(E::remove(index, &ids))));
            *next_id -= count as u64;
        }
        median_op(samples)
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

        // Inserts run before removes so the rows added here are the ones
        // taken away, leaving the index back at `n_docs` for the reopen
        // cell below.
        let mut next_id = n_docs as u64;
        let (ins1_wall, ins1_rss) =
            timed_inserts::<E>(index, extra_1, INPLACE_SINGLE, &mut next_id);
        let (ins100_wall, ins100_rss) =
            timed_inserts::<E>(index, extra_100, INPLACE_BATCH, &mut next_id);
        let (rem100_wall, rem100_rss) = timed_removes::<E>(index, INPLACE_BATCH, &mut next_id);
        let (rem1_wall, rem1_rss) = timed_removes::<E>(index, INPLACE_SINGLE, &mut next_id);
        assert_eq!(
            next_id, n_docs as u64,
            "lifecycle inserts and removes must balance so the reopen cell \
             sees the index it started from"
        );

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
                   path — table `append`/`delete` is a separate cell. Mutation cells \
                   discard a warm-up op, then report the median of five timed \
                   repetitions: the first mutation of a fresh index pays one-time \
                   cache-rebuild costs that are not a per-op cost."
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
        // [`CURVE_QUERIES`] the k=1 quantum is 0.001 and the k=10 quantum
        // 0.0001, which is finer than the differences being compared.
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
        // Infino through the PUBLIC API: a table built by the standard
        // lifecycle (append → commit → optimize) serving whatever
        // `vector.search_mode` the process config selects. The runner
        // points XDG_CONFIG_HOME at a config with `search_mode: flat_ivf`
        // for this cell, so the row is the shipped resident-plane mode —
        // reproducible by any user with one YAML line. The row is labeled
        // from the config so a default-config invocation can never
        // mislabel an ivf-served table as flat.
        let infino_mode = match engine_config().vector.search_mode {
            VectorSearchMode::FlatIvf => "infino-flat_ivf",
            VectorSearchMode::HnswIvf => "infino-hnsw_ivf",
            VectorSearchMode::Ivf => "infino-ivf",
        };
        let (infino_table, _infino_storage, _infino_dir) =
            supertable::build_local_for_serving(&prepared, n_docs);
        let infino_resident = supertable::served_index_blob_bytes(&infino_table);
        if infino_mode != "infino-ivf" {
            // A resident-mode config whose drain declined (register floor,
            // scale ceiling) silently serves ivf; publishing that as the
            // resident mode would be the exact mislabeling this arm exists
            // to avoid. Fail the run instead.
            assert!(
                infino_resident.is_some(),
                "{infino_mode} requested but no resident index was published \
                 (register floor or scale ceiling declined the build)"
            );
        }
        let infino_id_map = Arc::new(corpus::engine_id_to_dense(&infino_table, n_docs));
        let infino_read = exec_vec::SupertableVectorRead {
            table: &infino_table,
            id_to_dense: infino_id_map,
        };
        eprintln!(
            "[codec-curve] {infino_mode} serving: {}",
            infino_read.routing_label(exec_vec::ENGINE_DEFAULT, exec_vec::ENGINE_DEFAULT)
        );
        let infino_rows = per_k_rows(
            &infino_read,
            &q_corr,
            &gt_deep,
            infino_resident.unwrap_or(0) as usize,
        );
        // No build row for the table arm: its build is a durable ingest +
        // optimize, priced by the ingest and writes cells — a RAM-object
        // build number here would compare a commit against a memcpy.
        #[allow(unused_mut)] // mutated only when the `faiss` feature is on
        let mut curve_engines: Vec<(&str, Vec<CodecKRow>)> = vec![
            (infino_mode, infino_rows),
            ("turbovec-4bit", tv4_rows),
            (
                "turbovec-2bit",
                per_k_rows(&tv2, &q_corr, &gt_deep, tv2.index_bytes()),
            ),
        ];
        #[cfg(feature = "faiss")]
        {
            // FastScan (`PQ<M>x4fs`) is not run: at this per-coordinate
            // sub-quantizer count its recall is unstable across corpus
            // sizes, so published rows report classic PQ only. The
            // adapter stays in the lib for anyone measuring it directly.
            use retrievalbench::FaissPqVectorEngine;
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
    }
}

pub mod sql {
    use super::*;

    pub fn run(build: bool, warm: bool, cold: bool) {
        if let Err(reason) = tiers::supertable_backend_check() {
            eprintln!("[comparison-supertable-sql] skipped: {reason}");
            return;
        }
        // Infino: infino's own supertable SQL bench cell, verbatim.
        st_bench::sql::run(Phases { build, warm, cold });
    }
}
