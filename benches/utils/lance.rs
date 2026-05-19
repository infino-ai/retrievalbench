//! Shared helpers for Lance head-to-head benches: builds a Lance
//! `Table` from the same `Vec<f32>` corpus we hand to infino, runs
//! parameterized kNN searches, and calibrates each engine's
//! `(probe, refine)` knob to the lowest p50 latency that hits a
//! given recall@10 target.
//!
//! Lance is async; bench loops are sync, so callers pass a
//! `tokio::runtime::Runtime` and we `block_on` each call. One runtime
//! shared across the whole bench keeps the cost off the hot path.
//!
//! Lance was compiled against arrow 57 while the infino tree pulls
//! arrow 58. The bench reaches arrow 57 through the renamed
//! `arrow_array_lance` / `arrow_schema_lance` deps so we can hand
//! Lance the batch shape it expects.

#![allow(dead_code)]
// `calibrate_*` take a (reader, queries, truths, target_recall, probes,
// refines, p50_iter, k) tuple — all genuinely independent knobs of the
// sweep. Bundling them into a struct adds construction noise without
// clarity gain in a bench harness.
#![allow(clippy::too_many_arguments)]

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use arrow_array_lance::{
    Array, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, RecordBatchReader,
    UInt32Array,
};
use arrow_schema_lance::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::DistanceType;
use lancedb::Table;
use lancedb::index::Index;
use lancedb::index::vector::IvfPqIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase};
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand_distr::{Distribution, StandardNormal};
use tokio::runtime::Runtime;

use infino::superfile::vector::distance::normalize;
use infino::superfile::vector::reader::VectorReader;

pub const DIM: usize = 384;

/// One f32 distance + the local doc_id Lance returned.
pub type Hit = (u32, f32);

/// Build the Lance table at `path` with an IVF-PQ index, return it
/// open and ready for queries. Times the whole pipeline (data load
/// + IVF train + index write).
pub fn build_lance_table(
    rt: &Runtime,
    path: &Path,
    vectors: &[f32],
    n_docs: usize,
    n_partitions: u32,
    n_sub_vectors: u32,
) -> (Table, std::time::Duration) {
    let t0 = Instant::now();
    let table = rt.block_on(async move {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::UInt32, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    DIM as i32,
                ),
                false,
            ),
        ]));

        // One RecordBatch over the whole corpus. Lance will fragment
        // internally; we don't need to chunk on the bench side.
        let ids = UInt32Array::from((0..n_docs as u32).collect::<Vec<_>>());
        let flat = Float32Array::from(vectors.to_vec());
        let item_field = Arc::new(Field::new("item", DataType::Float32, true));
        let fsl = FixedSizeListArray::try_new(item_field, DIM as i32, Arc::new(flat), None)
            .expect("build FixedSizeListArray");
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(fsl)])
            .expect("build RecordBatch");
        let reader: Box<dyn RecordBatchReader + Send> =
            Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));

        let db = lancedb::connect(path.to_str().expect("path to_str"))
            .execute()
            .await
            .expect("await async result");
        let table = db
            .create_table("v", reader)
            .execute()
            .await
            .expect("create lance table");

        table
            .create_index(
                &["vector"],
                Index::IvfPq(
                    IvfPqIndexBuilder::default()
                        .num_partitions(n_partitions)
                        .num_sub_vectors(n_sub_vectors)
                        .distance_type(DistanceType::Cosine),
                ),
            )
            .execute()
            .await
            .expect("await async result");
        table
    });
    (table, t0.elapsed())
}

/// One Lance kNN call. Returns `(id, distance)` pairs sorted by
/// distance ascending — same shape as `infino::VectorReader::search`.
pub fn search_lance(
    rt: &Runtime,
    table: &Table,
    query: &[f32],
    k: usize,
    nprobes: usize,
    refine_factor: u32,
) -> Vec<Hit> {
    rt.block_on(async move {
        let q = query.to_vec();
        let stream = table
            .query()
            .nearest_to(q)
            .expect("nearest_to")
            .nprobes(nprobes)
            .refine_factor(refine_factor)
            .limit(k)
            .execute()
            .await
            .expect("await async result");
        let batches: Vec<RecordBatch> = stream.try_collect().await.expect("collect stream");
        let mut out = Vec::with_capacity(k);
        for b in batches {
            let id_col = b
                .column_by_name("id")
                .expect("column by name")
                .as_any()
                .downcast_ref::<UInt32Array>()
                .expect("downcast");
            let dist_col = b
                .column_by_name("_distance")
                .expect("column by name")
                .as_any()
                .downcast_ref::<Float32Array>()
                .expect("downcast");
            for i in 0..b.num_rows() {
                out.push((id_col.value(i), dist_col.value(i)));
            }
        }
        out
    })
}

/// Brute-force kNN ground truth for cosine distance on L2-normalized
/// vectors. Returns the top-k local doc_ids (no distances — recall
/// only needs the id set).
pub fn brute_force_topk_cosine(
    vectors: &[f32],
    n_docs: usize,
    query: &[f32],
    k: usize,
) -> Vec<u32> {
    assert_eq!(vectors.len(), n_docs * DIM);
    assert_eq!(query.len(), DIM);
    // For L2-normalized inputs cosine distance is monotone in -dot,
    // so we can rank by negative dot product.
    let mut scored: Vec<(u32, f32)> = (0..n_docs as u32)
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

/// Recall@k between a predicted top-k id list and ground truth.
pub fn recall_at_k(predicted: &[Hit], truth: &[u32]) -> f32 {
    if truth.is_empty() {
        return 1.0;
    }
    let truth_set: std::collections::HashSet<u32> = truth.iter().copied().collect();
    let hits = predicted
        .iter()
        .filter(|(id, _)| truth_set.contains(id))
        .count();
    hits as f32 / truth.len() as f32
}

/// Generate `n_queries` deterministic Gaussian queries, normalized.
/// Used only for smoke wiring; real benches should use
/// [`generate_realistic_queries`] so recall is meaningful at modest
/// nprobe.
pub fn generate_queries(n_queries: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = StdRng::seed_from_u64(seed);
    let dist = StandardNormal;
    (0..n_queries)
        .map(|_| {
            let mut q: Vec<f32> = (0..DIM)
                .map(|_| {
                    let s: f64 = dist.sample(&mut rng);
                    (s as f32) * 3.0
                })
                .collect();
            normalize(&mut q);
            q
        })
        .collect()
}

/// Pick `n_queries` corpus members and perturb each by small Gaussian
/// noise. A pure-Gaussian query lands far from any doc in
/// embedding space, so the top-10 NN are spread across many planted
/// clusters and IVF recall stays low even at high nprobe — this is
/// the same pattern the existing `tests/recall.rs` uses.
pub fn generate_realistic_queries(
    vectors: &[f32],
    n_docs: usize,
    n_queries: usize,
    seed: u64,
    normalize_each: bool,
    sigma: f32,
) -> Vec<Vec<f32>> {
    let mut rng = StdRng::seed_from_u64(seed);
    let dist = StandardNormal;
    let mut out = Vec::with_capacity(n_queries);
    for i in 0..n_queries {
        // Skip across the corpus by a coprime stride so consecutive
        // queries don't all sit in the first cluster.
        let base_idx = (i * 7919) % n_docs;
        let off = base_idx * DIM;
        let mut q: Vec<f32> = (0..DIM)
            .map(|d| {
                let s: f64 = dist.sample(&mut rng);
                vectors[off + d] + (s as f32) * sigma
            })
            .collect();
        if normalize_each {
            normalize(&mut q);
        }
        out.push(q);
    }
    out
}

/// Brute-force ground truth for a query batch.
pub fn ground_truth(
    vectors: &[f32],
    n_docs: usize,
    queries: &[Vec<f32>],
    k: usize,
) -> Vec<Vec<u32>> {
    queries
        .iter()
        .map(|q| brute_force_topk_cosine(vectors, n_docs, q, k))
        .collect()
}

/// Mean recall for one (engine, config) point over a query batch.
pub fn mean_recall_infino(
    reader: &VectorReader,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
    k: usize,
    nprobe: usize,
    rerank_mult: usize,
) -> f32 {
    let mut sum = 0f32;
    for (q, t) in queries.iter().zip(truths) {
        let hits = reader
            .search("v", q, k, nprobe, rerank_mult)
            .expect("FTS search");
        sum += recall_at_k(&hits, t);
    }
    sum / queries.len() as f32
}

pub fn mean_recall_lance(
    rt: &Runtime,
    table: &Table,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
    k: usize,
    nprobes: usize,
    refine_factor: u32,
) -> f32 {
    let mut sum = 0f32;
    for (q, t) in queries.iter().zip(truths) {
        let hits = search_lance(rt, table, q, k, nprobes, refine_factor);
        sum += recall_at_k(&hits, t);
    }
    sum / queries.len() as f32
}

/// p50 wall time over `n_iter` repetitions of one closure. Accepts
/// any `FnMut()` so it can wrap either engine's search call.
pub fn p50_micros<F: FnMut()>(mut f: F, n_iter: usize) -> f32 {
    let mut samples = Vec::with_capacity(n_iter);
    for _ in 0..n_iter {
        let t0 = Instant::now();
        f();
        samples.push(t0.elapsed().as_secs_f32() * 1e6);
    }
    samples.sort_unstable_by(|a, b| a.partial_cmp(b).expect("partial_cmp"));
    samples[samples.len() / 2]
}

/// Calibration result for one engine at one recall target.
#[derive(Debug, Clone, Copy)]
pub struct Calibrated {
    pub probe: usize,
    pub refine: usize,
    pub recall: f32,
    pub p50_micros: f32,
}

/// Sweep a (probe, refine) grid for infino, return the lowest-p50
/// point that hits ≥ `target_recall`. Returns `None` if no point in
/// the grid meets the target.
pub fn calibrate_infino(
    reader: &VectorReader,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
    target_recall: f32,
    probes: &[usize],
    refines: &[usize],
    p50_iter: usize,
    k: usize,
) -> Option<Calibrated> {
    let mut best: Option<Calibrated> = None;
    let mut peak_recall = 0f32;
    for &probe in probes {
        for &refine in refines {
            let recall = mean_recall_infino(reader, queries, truths, k, probe, refine);
            if recall > peak_recall {
                peak_recall = recall;
            }
            if recall < target_recall {
                continue;
            }
            // Use the first query as the timing fixture; Gaussian
            // queries are statistically interchangeable so p50 over
            // n_iter on one query approximates the mean shape.
            let q = &queries[0];
            let p50 = p50_micros(
                || {
                    let _ = reader.search("v", q, k, probe, refine).expect("FTS search");
                },
                p50_iter,
            );
            let cand = Calibrated {
                probe,
                refine,
                recall,
                p50_micros: p50,
            };
            best = match best {
                None => Some(cand),
                Some(b) if cand.p50_micros < b.p50_micros => Some(cand),
                Some(b) => Some(b),
            };
        }
    }
    if best.is_none() {
        eprintln!(
            "    [infino] no point hit recall ≥ {target_recall:.2}; peak observed = {peak_recall:.3}"
        );
    }
    best
}

pub fn calibrate_lance(
    rt: &Runtime,
    table: &Table,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
    target_recall: f32,
    probes: &[usize],
    refines: &[u32],
    p50_iter: usize,
    k: usize,
) -> Option<Calibrated> {
    let mut best: Option<Calibrated> = None;
    let mut peak_recall = 0f32;
    for &probe in probes {
        for &refine in refines {
            let recall = mean_recall_lance(rt, table, queries, truths, k, probe, refine);
            if recall > peak_recall {
                peak_recall = recall;
            }
            if recall < target_recall {
                continue;
            }
            let q = &queries[0];
            let p50 = p50_micros(
                || {
                    let _ = search_lance(rt, table, q, k, probe, refine);
                },
                p50_iter,
            );
            let cand = Calibrated {
                probe,
                refine: refine as usize,
                recall,
                p50_micros: p50,
            };
            best = match best {
                None => Some(cand),
                Some(b) if cand.p50_micros < b.p50_micros => Some(cand),
                Some(b) => Some(b),
            };
        }
    }
    if best.is_none() {
        eprintln!(
            "    [lance] no point hit recall ≥ {target_recall:.2}; peak observed = {peak_recall:.3}"
        );
    }
    best
}

/// Default `num_sub_vectors` for IVF-PQ at our DIM. Matches Lance's
/// own recommended setting for dim=384: 64 sub-vectors of 6 dims
/// each, 8-bit codes (256 centroids per subvec). Finer-grained
/// quantization than dim/8 = 48 → marginally better recall per
/// probe at the cost of ~33% larger PQ codes (64 B vs 48 B / vec).
pub fn default_n_sub_vectors() -> u32 {
    64
}
