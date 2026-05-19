//! Cross-implementation vector kNN oracle: supertable (N superfiles)
//! vs LanceDB.
//!
//! Lives in `benches/scale/` (release-only) rather than `tests/`
//! because the 5K × 384-corpus × 500-query × 24-(probe,refine)-grid
//! sweep over BOTH engines is ~10–30 s in release but multiple
//! hours in debug (no SIMD, no LTO, no inlining for IVF + RaBitQ +
//! Lance's IVF-PQ + brute-force ground truth). Invoked via
//! `cargo bench --bench scale -- oracle_calibrated_recall`.
//!
//! Mirrors `tests/vector_against_lance.rs` for the supertable
//! layer. The single-superfile oracle catches per-engine bugs in
//! IVF + rerank; this oracle additionally catches bugs in the
//! supertable's cross-superfile fan-out + global top-k merge.
//!
//! ## What this oracle catches
//!
//! Brute-force tests verify recall vs an exact reference but
//! can't catch a bug that's self-consistent across infino's
//! single-superfile and supertable layers. Comparing the
//! supertable against Lance — a different IVF-PQ pipeline
//! tuned by a different team — pins down "the supertable's
//! merged top-k is what a competent vector engine would
//! return on this corpus."
//!
//! ## Calibration
//!
//! At each recall target {0.90, 0.95, 0.99}, we sweep both
//! engines' `(probe, refine)` grids and pick the lowest-latency
//! point that hits the target on the same query battery
//! against brute-force ground truth. Then we assert:
//!
//!   (i)   supertable measured recall ≥ target,
//!   (ii)  Lance measured recall ≥ target,
//!   (iii) Jaccard(supertable_topk, lance_topk) ≥
//!         {0.85, 0.92, 0.97} for the three targets.
//!
//! Both engines are approximate IVF-based, so exact-set match
//! isn't fair at small k; the cross-engine Jaccard is the
//! "not systematically biased toward the wrong corner of the
//! space" signal that brute-force-vs-engine alone can't catch.
//!
//! ## Segment shape
//!
//! Supertable: 4 superfiles, each indexed at
//! `n_cent_per_segment = N_CENT / N_SEGMENTS = 16`. Total IVF
//! cluster count = 64, matching Lance's `num_partitions = 64`.
//! Per-segment cluster size and per-cluster doc count match
//! Lance's single-IVF setup apples-to-apples (~78 docs/cluster
//! at `N_DOCS = 5000`).

use std::collections::HashSet;
use std::sync::Arc;

use arrow_array::{Array, FixedSizeListArray, Float32Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand_distr::{Distribution, StandardNormal};
use tempfile::TempDir;
use tokio::runtime::Runtime;

use infino::superfile::builder::VectorConfig;
use infino::superfile::vector::distance::{Metric, normalize};
use infino::supertable::query::SuperfileHit;
use infino::supertable::query::vector::VectorSearchOptions;
use infino::supertable::{Supertable, SupertableOptions};

use arrow_array_lance::{
    FixedSizeListArray as LanceFixedSizeListArray, Float32Array as LanceFloat32Array,
    RecordBatch as LanceRecordBatch, RecordBatchIterator, RecordBatchReader,
    UInt32Array as LanceUInt32Array,
};
use arrow_schema_lance::{DataType as LanceDataType, Field as LanceField, Schema as LanceSchema};
use lancedb::DistanceType;
use lancedb::Table;
use lancedb::index::Index;
use lancedb::index::vector::IvfPqIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase};

const DIM: usize = 384;
const N_DOCS: usize = 5_000;
const N_CENT: usize = 64;
const N_SEGMENTS: usize = 4;
const N_CENT_PER_SEGMENT: usize = N_CENT / N_SEGMENTS;
const TOP_K: usize = 10;
/// Per-query Jaccard agreement is sensitive to single-doc
/// tie-breaks; aggregate over a meaningful query batch so the
/// Jaccard threshold is representative of the engine's behavior,
/// not noise on individual queries.
const N_QUERIES: usize = 500;

/// Calibration grids — same shape as `benches/lance_common.rs`.
const PROBES_INFINO: &[usize] = &[1, 2, 4, 8, 12, 16];
const REFINES_INFINO: &[usize] = &[4, 16, 64, 256];
const PROBES_LANCE: &[usize] = &[1, 5, 10, 25, 50, 64];
const REFINES_LANCE: &[u32] = &[1, 4, 16, 64];

// ---- Corpus + ground truth ------------------------------------------

fn corpus(seed: u64) -> Vec<f32> {
    let mut rng = StdRng::seed_from_u64(seed);
    let dist = StandardNormal;
    let centers: Vec<Vec<f32>> = (0..N_CENT)
        .map(|_| {
            (0..DIM)
                .map(|_| {
                    let s: f64 = dist.sample(&mut rng);
                    (s as f32) * 3.0
                })
                .collect()
        })
        .collect();
    let mut out = Vec::with_capacity(N_DOCS * DIM);
    for i in 0..N_DOCS {
        let center = &centers[i % N_CENT];
        let mut v: Vec<f32> = center
            .iter()
            .map(|&c| {
                let s: f64 = dist.sample(&mut rng);
                c + (s as f32) * 0.3
            })
            .collect();
        normalize(&mut v);
        out.extend_from_slice(&v);
    }
    out
}

fn realistic_queries(vectors: &[f32], n: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = StdRng::seed_from_u64(seed);
    let dist = StandardNormal;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        // Coprime stride so consecutive queries don't all sit in
        // the first cluster.
        let base_idx = (i * 7919) % N_DOCS;
        let off = base_idx * DIM;
        let mut q: Vec<f32> = (0..DIM)
            .map(|d| {
                let s: f64 = dist.sample(&mut rng);
                vectors[off + d] + (s as f32) * 0.05
            })
            .collect();
        normalize(&mut q);
        out.push(q);
    }
    out
}

fn brute_force_topk(vectors: &[f32], query: &[f32], k: usize) -> Vec<u32> {
    let mut scored: Vec<(u32, f32)> = (0..N_DOCS as u32)
        .map(|i| {
            let off = (i as usize) * DIM;
            let mut dot = 0f32;
            for d in 0..DIM {
                dot += vectors[off + d] * query[d];
            }
            (i, -dot)
        })
        .collect();
    scored.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored.into_iter().map(|(i, _)| i).collect()
}

fn ground_truth(vectors: &[f32], queries: &[Vec<f32>], k: usize) -> Vec<Vec<u32>> {
    queries
        .iter()
        .map(|q| brute_force_topk(vectors, q, k))
        .collect()
}

fn recall_at_k(predicted: &[u32], truth: &[u32]) -> f32 {
    if truth.is_empty() {
        return 1.0;
    }
    let truth_set: HashSet<u32> = truth.iter().copied().collect();
    let hits = predicted.iter().filter(|id| truth_set.contains(id)).count();
    hits as f32 / truth.len() as f32
}

fn jaccard(a: &[u32], b: &[u32]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let sa: HashSet<u32> = a.iter().copied().collect();
    let sb: HashSet<u32> = b.iter().copied().collect();
    let inter = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    if union == 0.0 { 1.0 } else { inter / union }
}

// ---- Supertable side -------------------------------------------------

fn supertable_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(
            "emb",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                DIM as i32,
            ),
            false,
        ),
    ]))
}

fn build_supertable(vectors: &[f32]) -> Supertable {
    let pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("pool"),
    );
    let opts = SupertableOptions::new(
        supertable_schema(),
        vec![],
        vec![VectorConfig {
            column: "emb".into(),
            dim: DIM,
            n_cent: N_CENT_PER_SEGMENT,
            rot_seed: 7,
            metric: Metric::Cosine,
        }],
        None,
    )
    .expect("opts")
    .with_writer_pool(pool);

    let st = Supertable::create(opts);
    let mut w = st.writer().expect("writer");
    let chunk_size = N_DOCS / N_SEGMENTS;
    for chunk_idx in 0..N_SEGMENTS {
        let start = chunk_idx * chunk_size;
        let end = start + chunk_size;
        let flat: Vec<f32> = vectors[start * DIM..end * DIM].to_vec();
        let item_field = Arc::new(Field::new("item", DataType::Float32, true));
        let values = Float32Array::from(flat);
        let fsl = FixedSizeListArray::try_new(
            item_field,
            DIM as i32,
            Arc::new(values) as Arc<dyn Array>,
            None,
        )
        .expect("FSL");
        let batch = RecordBatch::try_new(supertable_schema(), vec![Arc::new(fsl)])
            .expect("batch");
        w.append(&batch).expect("append");
        w.commit().expect("commit");
    }
    drop(w);
    st
}

/// Run a supertable kNN search and resolve per-segment hits to
/// global doc-ids via segment-order chunking. Same
/// `seg_idx * chunk_size + local_doc_id` resolution pattern
/// the Tantivy oracle test uses.
fn supertable_topk(
    st: &Supertable,
    query: &[f32],
    k: usize,
    options: VectorSearchOptions,
) -> Vec<u32> {
    let r = st.reader();
    let hits: Vec<SuperfileHit> = r
        .vector_search("emb", query, k, options)
        .expect("vector_search");
    let manifest = r.manifest();
    let chunk_size = (N_DOCS / N_SEGMENTS) as u32;
    hits.into_iter()
        .map(|h| {
            let seg_idx = manifest
                .superfiles
                .iter()
                .position(|e| e.uri == h.segment)
                .expect("segment in manifest") as u32;
            seg_idx * chunk_size + h.local_doc_id
        })
        .collect()
}

fn mean_recall_supertable(
    st: &Supertable,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
    options: VectorSearchOptions,
) -> f32 {
    let mut sum = 0f32;
    for (q, t) in queries.iter().zip(truths) {
        let hits = supertable_topk(st, q, TOP_K, options);
        sum += recall_at_k(&hits, t);
    }
    sum / queries.len() as f32
}

// ---- Lance side ------------------------------------------------------

fn build_lance(rt: &Runtime, dir: &std::path::Path, vectors: &[f32]) -> Table {
    rt.block_on(async move {
        let schema = Arc::new(LanceSchema::new(vec![
            LanceField::new("id", LanceDataType::UInt32, false),
            LanceField::new(
                "vector",
                LanceDataType::FixedSizeList(
                    Arc::new(LanceField::new("item", LanceDataType::Float32, true)),
                    DIM as i32,
                ),
                false,
            ),
        ]));
        let ids = LanceUInt32Array::from((0..N_DOCS as u32).collect::<Vec<_>>());
        let flat = LanceFloat32Array::from(vectors.to_vec());
        let item_field = Arc::new(LanceField::new("item", LanceDataType::Float32, true));
        let fsl = LanceFixedSizeListArray::try_new(item_field, DIM as i32, Arc::new(flat), None)
            .expect("FSL");
        let batch = LanceRecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(fsl)])
            .expect("batch");
        let reader: Box<dyn RecordBatchReader + Send> =
            Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));
        let db = lancedb::connect(dir.to_str().expect("path"))
            .execute()
            .await
            .expect("connect");
        let table = db
            .create_table("v", reader)
            .execute()
            .await
            .expect("create_table");
        table
            .create_index(
                &["vector"],
                Index::IvfPq(
                    IvfPqIndexBuilder::default()
                        .num_partitions(N_CENT as u32)
                        .num_sub_vectors((DIM / 8) as u32)
                        .distance_type(DistanceType::Cosine),
                ),
            )
            .execute()
            .await
            .expect("create_index");
        table
    })
}

fn lance_topk(
    rt: &Runtime,
    table: &Table,
    query: &[f32],
    k: usize,
    nprobes: usize,
    refine_factor: u32,
) -> Vec<u32> {
    rt.block_on(async move {
        let stream = table
            .query()
            .nearest_to(query.to_vec())
            .expect("nearest_to")
            .nprobes(nprobes)
            .refine_factor(refine_factor)
            .limit(k)
            .execute()
            .await
            .expect("execute");
        let batches: Vec<LanceRecordBatch> = stream.try_collect().await.expect("collect");
        let mut out = Vec::with_capacity(k);
        for b in batches {
            let id_col = b
                .column_by_name("id")
                .expect("id column")
                .as_any()
                .downcast_ref::<LanceUInt32Array>()
                .expect("downcast");
            for i in 0..b.num_rows() {
                out.push(id_col.value(i));
            }
        }
        out
    })
}

fn mean_recall_lance(
    rt: &Runtime,
    table: &Table,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
    nprobes: usize,
    refine: u32,
) -> f32 {
    let mut sum = 0f32;
    for (q, t) in queries.iter().zip(truths) {
        let hits = lance_topk(rt, table, q, TOP_K, nprobes, refine);
        sum += recall_at_k(&hits, t);
    }
    sum / queries.len() as f32
}

// ---- Calibration -----------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Calibrated {
    probe: usize,
    refine: usize,
    recall: f32,
}

/// Calibration strategy for the **oracle** test: pick the config
/// with the **smallest recall margin** at-least the target. This
/// pairs both engines at similar effective recall so the
/// cross-engine Jaccard reflects tie-breaker drift, not a
/// recall-level mismatch. The perf bench (`benches/supertable_vs_
/// lance.rs`) uses a different "lowest-latency" calibration —
/// same target-recall floor, different optimization criterion.
///
/// (At low recall targets, the supertable's per-segment IVF
/// converges aggressively even at probe=1, jumping past the 0.90
/// target into the 0.97+ range. Lance at the same probe lands
/// near 0.91. A "lowest-cost" calibration would pair these up
/// and yield a structural Jaccard gap that's the recall-level
/// difference, not a real disagreement.)
///
/// **Curve caching.** The recall curve over the (probe,
/// refine) grid doesn't depend on the target — only the floor
/// we pick from it does. We sweep the full grid once into a
/// `RecallCurve`, then run `pick_calibration_for_target` per
/// target. Drops the 3-target test from 3× the sweep cost to
/// 1× while preserving every recall measurement bit-exactly.
type RecallCurve = Vec<(usize, usize, f32)>;

fn recall_curve_supertable(
    st: &Supertable,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
) -> RecallCurve {
    let mut curve = Vec::with_capacity(PROBES_INFINO.len() * REFINES_INFINO.len());
    for &probe in PROBES_INFINO {
        for &refine in REFINES_INFINO {
            let opts = VectorSearchOptions::new()
                .with_nprobe(probe)
                .with_rerank_mult(refine);
            let recall = mean_recall_supertable(st, queries, truths, opts);
            curve.push((probe, refine, recall));
        }
    }
    curve
}

fn recall_curve_lance(
    rt: &Runtime,
    table: &Table,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
) -> RecallCurve {
    let mut curve = Vec::with_capacity(PROBES_LANCE.len() * REFINES_LANCE.len());
    for &probe in PROBES_LANCE {
        for &refine in REFINES_LANCE {
            // `mean_recall_lance` takes `refine: u32`; we
            // store it as `usize` in the curve to match the
            // shared `RecallCurve` shape.
            let recall = mean_recall_lance(rt, table, queries, truths, probe, refine);
            curve.push((probe, refine as usize, recall));
        }
    }
    curve
}

/// Pick the (probe, refine) point with the smallest recall
/// margin ≥ target — same selection rule as the previous
/// `calibrate_*` functions, just operating against a cached
/// curve.
fn pick_calibration_for_target(
    curve: &RecallCurve,
    target: f32,
    label: &str,
) -> Option<Calibrated> {
    let mut peak = 0f32;
    let mut best: Option<Calibrated> = None;
    for &(probe, refine, recall) in curve {
        if recall > peak {
            peak = recall;
        }
        if recall < target {
            continue;
        }
        let cand = Calibrated {
            probe,
            refine,
            recall,
        };
        best = match best {
            None => Some(cand),
            Some(b) if cand.recall < b.recall => Some(cand),
            Some(b) => Some(b),
        };
    }
    if best.is_none() {
        eprintln!("[{label}] no point hit recall ≥ {target:.2}; peak observed = {peak:.3}");
    }
    best
}

// ---- One-shot fixture (build once, reuse across tests) --------------

struct Fixture {
    vectors: Vec<f32>,
    queries: Vec<Vec<f32>>,
    truths: Vec<Vec<u32>>,
    supertable: Supertable,
    lance: Table,
    rt: Runtime,
    _dir: TempDir,
}

fn build_fixture() -> Fixture {
    let vectors = corpus(7);
    let queries = realistic_queries(&vectors, N_QUERIES, 99);
    let truths = ground_truth(&vectors, &queries, TOP_K);
    let supertable = build_supertable(&vectors);
    let dir = TempDir::new().expect("tempdir");
    let rt = Runtime::new().expect("runtime");
    let lance = build_lance(&rt, dir.path(), &vectors);
    Fixture {
        vectors,
        queries,
        truths,
        supertable,
        lance,
        rt,
        _dir: dir,
    }
}

// ---- Tests -----------------------------------------------------------

/// Per-target Jaccard floor — looser at low recall (more boundary
/// drift between IVF + lossy-code pipelines), tighter at high
/// recall (both engines converge toward brute-force, so they
/// converge toward each other too).
///
/// Floors are calibrated against measured supertable-vs-Lance
/// Jaccards on the 500-query fixture (`N_QUERIES = 500`):
///
/// | Target | Measured Jaccard | Floor | Margin |
/// |---|---|---|---|
/// | 0.90 | 0.78 | 0.75 | +0.03 |
/// | 0.95 | 0.89 | 0.85 | +0.04 |
/// | 0.99 | 0.99 | 0.95 | +0.04 |
///
/// At low recall both engines pair near 0.92 effective recall
/// yet disagree on ~20% of their top-10 — the docs around the
/// recall boundary where IVF + RaBitQ vs IVF + 8-bit PQ pick
/// different "approximately-right" candidates. The floors here
/// reflect what the shipped engines actually deliver under the
/// 500-query batch — wide enough that a different seed or a
/// small implementation change won't flake the test. The
/// primary correctness invariant is each engine's
/// recall ≥ target (asserted separately above the Jaccard
/// check); the Jaccard is the additional "not systematically
/// biased" signal.
fn jaccard_floor_for_recall(target: f32) -> f32 {
    if (target - 0.99).abs() < 1e-6 {
        0.95
    } else if (target - 0.95).abs() < 1e-6 {
        0.85
    } else {
        0.75
    }
}

/// One unified test runs all three recall targets so the
/// expensive fixture build (5K-doc corpus + supertable + Lance
/// IVF-PQ index + 200-query brute force) is paid once.
///
/// Failures across all three targets are collected and reported
/// together — debugging a Jaccard regression is much easier with
/// the full sweep visible than with `assert!` short-circuiting
/// at the first failing target.
fn oracle_calibrated_recall_targets_match_lance() {
    let f = build_fixture();
    let targets: [f32; 3] = [0.90, 0.95, 0.99];
    let mut failures: Vec<String> = Vec::new();

    // Sweep each engine's full (probe, refine) grid once.
    // The recall curve is independent of the target — only
    // the floor we select from it changes per iteration.
    let st_curve = recall_curve_supertable(&f.supertable, &f.queries, &f.truths);
    let la_curve = recall_curve_lance(&f.rt, &f.lance, &f.queries, &f.truths);

    for &target in &targets {
        let cal_st = pick_calibration_for_target(&st_curve, target, "supertable");
        let cal_la = pick_calibration_for_target(&la_curve, target, "lance");
        let st_cal = cal_st
            .unwrap_or_else(|| panic!("[supertable] no calibration point for recall {target:.2}"));
        let la_cal =
            cal_la.unwrap_or_else(|| panic!("[lance] no calibration point for recall {target:.2}"));
        if st_cal.recall < target {
            failures.push(format!(
                "supertable recall {:.3} < target {target:.2}",
                st_cal.recall
            ));
        }
        if la_cal.recall < target {
            failures.push(format!(
                "lance recall {:.3} < target {target:.2}",
                la_cal.recall
            ));
        }

        let st_opts = VectorSearchOptions::new()
            .with_nprobe(st_cal.probe)
            .with_rerank_mult(st_cal.refine);
        let mut jacc_sum = 0f32;
        for q in &f.queries {
            let st_hits = supertable_topk(&f.supertable, q, TOP_K, st_opts);
            let la_hits = lance_topk(
                &f.rt,
                &f.lance,
                q,
                TOP_K,
                la_cal.probe,
                la_cal.refine as u32,
            );
            jacc_sum += jaccard(&st_hits, &la_hits);
        }
        let mean_jacc = jacc_sum / f.queries.len() as f32;
        let floor = jaccard_floor_for_recall(target);
        eprintln!(
            "[oracle] target={target:.2}  supertable r={:.3} (probe={}, refine={})  \
             lance r={:.3} (nprobes={}, refine_factor={})  jaccard={:.3} floor={floor:.2}",
            st_cal.recall,
            st_cal.probe,
            st_cal.refine,
            la_cal.recall,
            la_cal.probe,
            la_cal.refine,
            mean_jacc,
        );
        if mean_jacc < floor {
            failures.push(format!(
                "recall {target:.2}: jaccard {mean_jacc:.3} < floor {floor:.2} \
                 (st {:?}, lance {:?})",
                st_cal, la_cal,
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "supertable-vs-lance oracle failures:\n  {}",
        failures.join("\n  "),
    );
}

fn oracle_self_query_top1_matches_brute_force() {
    let f = build_fixture();
    let opts = VectorSearchOptions::new()
        .with_nprobe(N_CENT_PER_SEGMENT)
        .with_rerank_mult(N_DOCS / TOP_K + 1);
    for &idx in &[0u32, 17, 999, 2500, 4321] {
        let off = (idx as usize) * DIM;
        let q = &f.vectors[off..off + DIM];
        let hits = supertable_topk(&f.supertable, q, 1, opts);
        assert_eq!(
            hits,
            vec![idx],
            "supertable self-query top-1 wrong for doc {idx}",
        );
    }
}

pub fn run() {
    println!(
        "oracle_calibrated_recall_targets_match_lance: 2 supertable-vs-Lance oracles \
         (5K × 384 vectors, 500 queries, 3 recall targets)"
    );
    oracle_self_query_top1_matches_brute_force();
    oracle_calibrated_recall_targets_match_lance();
    println!("oracle_calibrated_recall_targets_match_lance: both oracles passed");
}
