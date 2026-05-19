//! Cross-implementation vector kNN oracle: our vector pipeline vs LanceDB.
//!
//! Mirrors `tests/bm25_against_tantivy.rs` for the vector side. We
//! index the same synthetic corpus into both engines with matching
//! IVF cluster count and matching cosine metric, run kNN on a query
//! battery, and assert top-k agreement.
//!
//! ## What this oracle catches
//!
//! Brute-force oracle tests (`tests/recall.rs`,
//! `tests/vector_brute_force_oracle.rs`) verify recall vs an exact
//! reference but can't catch a self-consistent bug in our IVF +
//! RaBitQ pipeline that systematically misranks within the source
//! cluster — the brute-force oracle and the buggy infino path could
//! agree on planted-cluster data while disagreeing with every other
//! IVF implementation. Comparing against Lance's IVF-PQ pipeline at
//! a high-recall config catches this class.
//!
//! ## Calibration
//!
//! Both engines run at maximal-recall configs:
//!
//!   - infino: `nprobe == n_cent` (full sweep), `rerank_mult` large
//!     (the entire shortlist is reranked with f32).
//!   - Lance: `nprobes == num_partitions` (full sweep),
//!     `refine_factor` large.
//!
//! At these configs both engines should approach brute-force-level
//! recall on the planted-cluster corpus (recall@10 ≥ 0.95 in
//! practice), so disagreement between them indicates a real
//! correctness bug, not an IVF approximation artifact.
//!
//! ## Tolerances
//!
//! The strong invariant is **"both engines agree with brute-force
//! ground truth on the same query."** Engine-vs-engine direct
//! comparison is weaker because IVF + lossy-code pipelines can
//! tie-break within a cluster differently while both being correct
//! relative to the actual nearest-neighbor metric.
//!
//! - Self-query top-1: must equal the source doc (trivially, since
//!   `dot(self, self) = 1.0` is the maximum).
//! - Self-query top-3: each engine's top-3 must match brute force.
//! - Perturbed-query top-1: each engine's top-1 must match brute
//!   force.
//! - Aggregate top-10 Jaccard vs brute-force, averaged over a
//!   query battery, ≥ 0.90 for both engines.
//! - Engine-vs-engine top-10 Jaccard ≥ 0.85 on average — this is
//!   the "stay competitive with Lance" invariant.
//!
//! ## Why not test scores numerically
//!
//! Lance reports cosine distance (`1 - cos_sim`); infino reports
//! `1 - dot` for normalized inputs. Numerically identical for
//! unit-norm vectors but the engines compute via different SIMD
//! kernels, so float-equality on scores would flake. Top-k *order*
//! is invariant under monotone transforms, so the doc-set agreement
//! is the right invariant.

use bytes::Bytes;
use futures::TryStreamExt;
use infino::superfile::vector::builder::{VectorBuilder, VectorConfig};
use infino::superfile::vector::distance::{Metric, normalize};
use infino::superfile::vector::reader::VectorReader;
use lancedb::DistanceType;
use lancedb::Table;
use lancedb::index::Index;
use lancedb::index::vector::IvfPqIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase};
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand_distr::{Distribution, StandardNormal};
use std::collections::HashSet;
use std::sync::{Arc, LazyLock};
use tempfile::TempDir;
use tokio::runtime::Runtime;

use arrow_array_lance::{
    FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, RecordBatchReader,
    UInt32Array,
};
use arrow_schema_lance::{DataType, Field, Schema};

const DIM: usize = 384;
const N_DOCS: usize = 5_000;
const N_CENT: usize = 64;
const TOP_K: usize = 10;

// Maximal-recall configs: full IVF sweep + a rerank pool wide
// enough to cover every doc. With `rerank_mult * k ≥ N_DOCS` and
// nprobe == n_cent, every doc is in the rerank pool, so the engine
// is doing a full f32 brute-force scan dressed up as IVF — exactly
// the regime where any disagreement with brute force is a real
// correctness bug, not a recall artifact.
const INFINO_NPROBE_MAX: usize = N_CENT;
const INFINO_RERANK_MULT_MAX: usize = N_DOCS / TOP_K + 1;
const LANCE_NPROBES_MAX: usize = N_CENT;
const LANCE_REFINE_FACTOR_MAX: u32 = (N_DOCS / TOP_K + 1) as u32;

/// Planted-cluster corpus shape that the existing recall test uses.
/// Centers are NOT normalized so per-doc noise of 0.3 stays small
/// relative to the center magnitude (||center|| ≈ 58 vs ||noise||
/// ≈ 5.9 for sigma=0.3 at dim=384). Per-doc normalize is applied at
/// the end so we can use cosine distance.
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

/// Build infino's vector reader from the flat corpus.
fn build_infino(vectors: &[f32]) -> VectorReader {
    let mut b = VectorBuilder::new();
    b.register_column(VectorConfig {
        name: "v".into(),
        dim: DIM,
        n_cent: N_CENT,
        rot_seed: 7,
        metric: Metric::Cosine,
    })
    .expect("register column");
    for i in 0..N_DOCS {
        let off = i * DIM;
        b.add(0, &vectors[off..off + DIM])
            .expect("add to vector builder");
    }
    let blob = b.finish();
    let json =
        format!(r#"[{{"name":"v","dim":{DIM},"n_cent":{N_CENT},"rot_seed":7,"metric":"cosine"}}]"#);
    VectorReader::open(Bytes::from(blob), &json).expect("open VectorReader")
}

/// Build a Lance table at `dir` with IVF-PQ over the same corpus.
/// Returns the open `Table` plus the runtime (kept alive together
/// because async calls go through `block_on`).
fn build_lance(rt: &Runtime, dir: &std::path::Path, vectors: &[f32]) -> Table {
    rt.block_on(async move {
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
        let ids = UInt32Array::from((0..N_DOCS as u32).collect::<Vec<_>>());
        let flat = Float32Array::from(vectors.to_vec());
        let item_field = Arc::new(Field::new("item", DataType::Float32, true));
        let fsl = FixedSizeListArray::try_new(item_field, DIM as i32, Arc::new(flat), None)
            .expect("build FixedSizeListArray");
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(fsl)])
            .expect("build RecordBatch");
        let reader: Box<dyn RecordBatchReader + Send> =
            Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));

        let db = lancedb::connect(dir.to_str().expect("path to_str"))
            .execute()
            .await
            .expect("await async result");
        let table = db
            .create_table("v", reader)
            .execute()
            .await
            .expect("create lance table");
        // dim/8 sub-vectors at 8-bit each: same shape Lance suggests.
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
            .expect("await async result");
        table
    })
}

fn infino_top_k(reader: &VectorReader, query: &[f32], k: usize) -> Vec<u32> {
    reader
        .search("v", query, k, INFINO_NPROBE_MAX, INFINO_RERANK_MULT_MAX)
        .expect("search")
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

fn lance_top_k(rt: &Runtime, table: &Table, query: &[f32], k: usize) -> Vec<u32> {
    rt.block_on(async move {
        let stream = table
            .query()
            .nearest_to(query.to_vec())
            .expect("nearest_to")
            .nprobes(LANCE_NPROBES_MAX)
            .refine_factor(LANCE_REFINE_FACTOR_MAX)
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
            for i in 0..b.num_rows() {
                out.push(id_col.value(i));
            }
        }
        out
    })
}

/// Brute-force top-k by cosine on unit-norm vectors (sort by `-dot`).
fn brute_force_top_k(vectors: &[f32], query: &[f32], k: usize) -> Vec<u32> {
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

fn perturb_query(vectors: &[f32], idx: usize, rng: &mut StdRng, sigma: f32) -> Vec<f32> {
    let dist = StandardNormal;
    let off = idx * DIM;
    let mut q: Vec<f32> = (0..DIM)
        .map(|d| {
            let s: f64 = dist.sample(rng);
            vectors[off + d] + (s as f32) * sigma
        })
        .collect();
    normalize(&mut q);
    q
}

// ---- One-time fixture: build both indexes, share across tests --------
//
// The 5 tests in this file all need the same corpus +
// infino IVF + Lance IVF-PQ. Each engine build is expensive
// (k-means training, PQ codebook fitting). Building per-
// test means 5× the setup work. `LazyLock` builds the
// fixture on first access and lets every test share `&FIXTURE`.

struct Fixture {
    vectors: Vec<f32>,
    infino: VectorReader,
    lance: Table,
    rt: Runtime,
    _dir: TempDir,
}

fn build_fixture() -> Fixture {
    let vectors = corpus(7);
    let infino = build_infino(&vectors);
    let dir = TempDir::new().expect("create TempDir");
    let rt = Runtime::new().expect("new tokio runtime");
    let lance = build_lance(&rt, dir.path(), &vectors);
    Fixture {
        vectors,
        infino,
        lance,
        rt,
        _dir: dir,
    }
}

static FIXTURE: LazyLock<Fixture> = LazyLock::new(build_fixture);

// ---- Tests -----------------------------------------------------------

#[test]
fn oracle_self_query_top1_matches_lance() {
    // Query is a corpus member exactly. Top-1 must be that doc on
    // both engines.
    let f = &*FIXTURE;
    for &idx in &[0u32, 17, 123, 999, 4321] {
        let off = (idx as usize) * DIM;
        let q = &f.vectors[off..off + DIM];
        let inf = infino_top_k(&f.infino, q, 1);
        let lan = lance_top_k(&f.rt, &f.lance, q, 1);
        assert_eq!(inf, vec![idx], "infino self-query top1 wrong for doc {idx}");
        assert_eq!(lan, vec![idx], "lance self-query top1 wrong for doc {idx}");
    }
}

#[test]
fn oracle_self_query_top3_matches_brute_force() {
    // Top-3 of corpus[i] vs corpus[i] is brute-force-decidable: the
    // 3 closest doc IDs by cosine. Each engine's top-3 must equal
    // brute force. Lance and infino can break sub-tier ties
    // differently while both being correct relative to the metric;
    // brute force is the canonical answer.
    let f = &*FIXTURE;
    for &idx in &[0u32, 17, 1234, 4999] {
        let off = (idx as usize) * DIM;
        let q = &f.vectors[off..off + DIM];
        let truth: HashSet<u32> = brute_force_top_k(&f.vectors, q, 3).into_iter().collect();
        let inf: HashSet<u32> = infino_top_k(&f.infino, q, 3).into_iter().collect();
        let lan: HashSet<u32> = lance_top_k(&f.rt, &f.lance, q, 3).into_iter().collect();
        assert_eq!(
            inf, truth,
            "infino self-query top-3 disagrees with brute force for doc {idx}"
        );
        assert_eq!(
            lan, truth,
            "lance self-query top-3 disagrees with brute force for doc {idx}"
        );
    }
}

#[test]
fn oracle_perturbed_query_top1_matches_brute_force() {
    // Each engine's top-1 must match brute-force ground truth on
    // the same query — that's the correctness contract. Engine-vs-
    // engine top-1 disagreement is allowed when brute force itself
    // ranks two cluster-mates within float-tie-break range; what
    // matters is that each engine independently lands on the right
    // doc.
    let f = &*FIXTURE;
    let mut rng = StdRng::seed_from_u64(101);
    for &idx in &[5u32, 88, 511, 2048, 4099] {
        let q = perturb_query(&f.vectors, idx as usize, &mut rng, 0.005);
        let truth = brute_force_top_k(&f.vectors, &q, 1);
        let inf = infino_top_k(&f.infino, &q, 1);
        let lan = lance_top_k(&f.rt, &f.lance, &q, 1);
        assert_eq!(inf, truth, "infino top-1 wrong for source {idx}");
        assert_eq!(lan, truth, "lance top-1 wrong for source {idx}");
    }
}

#[test]
fn oracle_top10_jaccard_infino_vs_lance_aggregate() {
    // Stay-competitive invariant: across a battery of perturbed
    // queries, infino's top-10 and Lance's top-10 should overlap
    // heavily. Per-query disagreement is allowed (different
    // tie-break rules) but the aggregate must be ≥ 0.85.
    let f = &*FIXTURE;
    let mut rng = StdRng::seed_from_u64(202);
    let mut total = 0f32;
    let n_queries = 30usize;
    for q_idx in 0..n_queries {
        let src = (q_idx * 137) % N_DOCS;
        let q = perturb_query(&f.vectors, src, &mut rng, 0.05);
        let inf = infino_top_k(&f.infino, &q, TOP_K);
        let lan = lance_top_k(&f.rt, &f.lance, &q, TOP_K);
        total += jaccard(&inf, &lan);
    }
    let mean = total / n_queries as f32;
    assert!(
        mean >= 0.85,
        "mean Jaccard@10 infino-vs-lance below threshold: {mean:.3} < 0.85"
    );
}

#[test]
fn oracle_both_engines_close_to_brute_force() {
    // Sanity: at maximal config both engines should agree closely
    // with brute force. Catches the case where one engine is broken
    // but the two-way oracle agrees because they're broken the same
    // way (extremely unlikely given different code paths, but worth
    // pinning).
    let f = &*FIXTURE;
    let mut rng = StdRng::seed_from_u64(303);
    let n_queries = 20usize;
    let mut sum_inf = 0f32;
    let mut sum_lan = 0f32;
    for q_idx in 0..n_queries {
        let src = (q_idx * 191) % N_DOCS;
        let q = perturb_query(&f.vectors, src, &mut rng, 0.05);
        let truth = brute_force_top_k(&f.vectors, &q, TOP_K);
        let inf = infino_top_k(&f.infino, &q, TOP_K);
        let lan = lance_top_k(&f.rt, &f.lance, &q, TOP_K);
        sum_inf += jaccard(&inf, &truth);
        sum_lan += jaccard(&lan, &truth);
    }
    let mean_inf = sum_inf / n_queries as f32;
    let mean_lan = sum_lan / n_queries as f32;
    assert!(
        mean_inf >= 0.90,
        "infino mean Jaccard@10 vs brute force below threshold: {mean_inf:.3} < 0.90"
    );
    assert!(
        mean_lan >= 0.90,
        "lance mean Jaccard@10 vs brute force below threshold: {mean_lan:.3} < 0.90"
    );
}
