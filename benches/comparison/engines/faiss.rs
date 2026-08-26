// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! FAISS peer implementation of [`VectorEngine`].
//!
//! Two adapters, both product-quantized flat indexes built through
//! FAISS's `index_factory` string grammar (the crate exposes no typed
//! `IndexPQ` / `IndexPQFastScan` wrapper — see the factory-string note
//! below):
//!
//! - [`FaissPqFastScanVectorEngine`] — `PQ<M>x4fs`, the SIMD "fast scan"
//!   kernel. This is the search-speed cell: FAISS hard-locks fast-scan
//!   to `nbits=4` (the FastScan accumulation kernel only packs 4-bit
//!   codes), so this adapter's bit width is not a choice.
//! - [`FaissPqVectorEngine`] — `PQ<M>x8`, the plain LUT-based scanner
//!   (FAISS calls this the "LUT256" table-lookup path: an nbits=8 code
//!   selects one of 256 precomputed per-subvector distances). This is
//!   the recall cell: no SIMD packing constraint, so it runs at full
//!   quantization precision for its width.
//!
//! ## Why `index_factory` instead of typed wrappers
//!
//! `faiss-rs` 0.13.0's `index` module ships typed wrappers only for
//! `flat`, `ivf_flat`, `lsh`, `pretransform`, `refine_flat`, and
//! `scalar_quantizer` — there is no `pq` submodule. PQ and PQFastScan
//! are reachable only through the generic `index_factory(d, description,
//! metric) -> Result<IndexImpl>` entry point and FAISS's own factory
//! string grammar (`"PQ32x4fs"`, `"PQ16x8"`, …), so both adapters here
//! build an `IndexImpl` and drive it through the generic `Index` trait
//! rather than a PQ-specific type.
//!
//! ## Sizing `M` (sub-quantizer count)
//!
//! FAISS's PQ constructor takes `M` sub-quantizers, each covering
//! `d / M` dimensions, encoded at `nbits` bits — a different knob from
//! `sq4flat.rs` / `turboquant/vector.rs`, which both express bit width
//! as *bits per dimension* (one code per coordinate: `Sq4FlatIndex` at
//! 4 bits/dim, `turbovec-4bit`/`turbovec-2bit` at 4 and 2 bits/dim).
//! FAISS PQ has no bits-per-dimension mode; the closest equivalent is
//! `M = d` (one sub-quantizer per coordinate, sub-vector length 1),
//! which this adapter uses for both variants so the comparison sits at
//! the same density as its siblings: `PQ<d>x4fs` is 4 bits/dim (matching
//! `Sq4FlatIndex` and `turbovec-4bit` exactly), and `PQ<d>x8` is 8
//! bits/dim = 1 byte/dim (matching `Sq4FlatIndex`'s residual variant and
//! one step past `turbovec-4bit`). `d % M == 0` holds trivially since
//! `M == d`.
//!
//! ## Metric support
//!
//! FAISS's `MetricType` has exactly two variants, `InnerProduct` and
//! `L2` — there is no native `Cosine`. `Cosine` is supported here by
//! L2-normalizing every row (query and corpus alike) before handing it
//! to an `InnerProduct` index, the standard FAISS idiom for cosine
//! ranking; `L2Sq` maps to FAISS `L2`, and `NegDot` maps to
//! `InnerProduct` with the returned distance negated to match the
//! trait's smaller-is-better contract (mirroring the negation
//! `turboquant/vector.rs` already does for its own inner-product-native
//! library).
//!
//! ## `&mut self` search vs. the trait's `&self` receiver
//!
//! `Index::search` takes `&mut self` (FAISS's native search call is not
//! internally synchronized), but `VectorEngine::read` and
//! `VectorRead::topk_global` both hand back `&Self::Index`. `IndexImpl`
//! has no `Clone`, so the index is held behind a `RefCell` — safe here
//! because every adapter in this harness is driven single-threaded per
//! index (the same assumption `infino_vector_engine.rs`'s retained
//! `SuperfileReader` makes, just without needing a `RefCell` because its
//! search methods already take `&self`).
//!
//! ## No resident-byte accounting
//!
//! Unlike `Sq4FlatIndex`/`TurboQuantIndex`, `faiss-rs`'s `IndexImpl`
//! exposes no serialized-size or code-size accessor, so this adapter
//! cannot report an exact measured byte count the way the sibling
//! adapters do. It instead logs an *analytically computed* estimate
//! (`M * ceil(nbits / 8)` code bytes per row, the standard PQ code-size
//! formula, plus a `256 * dim` fp32 codebook) and says so in the log
//! line rather than presenting it as a measured figure.

use std::cell::RefCell;

use faiss::index::{IndexImpl, index_factory};
use faiss::{Idx, Index, MetricType};

use infino_bench_utils::executors::vector::{ENGINE_DEFAULT, VectorRead};
use infino_bench_utils::harness::{
    Capabilities, VectorEngine, VectorHit, VectorMetric, VectorSearch,
};
use infino_bench_utils::rss::fmt_bytes;

/// Bits per sub-quantizer code for the fast-scan variant. FAISS's
/// FastScan accumulation kernel only packs 4-bit codes — not a tuning
/// choice, a hard constraint of the SIMD kernel.
const FAST_SCAN_NBITS: u32 = 4;
/// Bits per sub-quantizer code for the plain (LUT256) variant, per the
/// task's recall-cell requirement.
const PLAIN_PQ_NBITS: u32 = 8;
/// Bytes per fp32 coordinate, used to size the analytical codebook
/// estimate in [`estimate_resident_bytes`].
const F32_BYTES: usize = 4;
/// PQ codebook size: 256 centroids per sub-quantizer, the maximum
/// addressable by an 8-bit code and the value FAISS uses whenever
/// `nbits <= 8` (both variants here).
const CODEBOOK_CENTROIDS: usize = 256;

/// Maps the harness's engine-agnostic metric to FAISS's `MetricType`.
/// FAISS has no native `Cosine`; callers normalize rows to unit length
/// before indexing/querying under `InnerProduct` instead (see
/// [`normalize_rows`] / [`normalize_one`]) — this function only picks
/// the FAISS-side enum, not whether normalization is required.
fn map_metric(metric: VectorMetric) -> MetricType {
    match metric {
        VectorMetric::L2Sq => MetricType::L2,
        VectorMetric::Cosine => MetricType::InnerProduct,
        VectorMetric::NegDot => MetricType::InnerProduct,
    }
}

/// L2-normalizes every row of a row-major `n × dim` buffer in place,
/// FAISS's standard idiom for scoring cosine distance through an
/// `InnerProduct` index. A zero row is left as-is (it has no direction
/// to normalize to; leaving it zero keeps its inner product zero
/// against everything, the least-wrong fallback).
fn normalize_rows(vectors: &mut [f32], dim: usize) {
    for row in vectors.chunks_exact_mut(dim) {
        normalize_one(row);
    }
}

/// L2-normalizes one row in place; see [`normalize_rows`].
fn normalize_one(row: &mut [f32]) {
    let norm = row.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in row.iter_mut() {
            *x /= norm;
        }
    }
}

/// Sub-quantizer count for both variants: one code per dimension. See
/// the module doc-comment's "Sizing `M`" section.
fn sub_quantizer_count(dim: usize) -> usize {
    dim
}

/// Factory description string for the fast-scan variant: `PQ<M>x4fs`.
fn fast_scan_description(dim: usize) -> String {
    format!("PQ{}x{FAST_SCAN_NBITS}fs", sub_quantizer_count(dim))
}

/// Factory description string for the plain LUT256 variant: `PQ<M>x8`.
fn plain_pq_description(dim: usize) -> String {
    format!("PQ{}x{PLAIN_PQ_NBITS}", sub_quantizer_count(dim))
}

/// Analytically estimated resident bytes — see the module doc-comment's
/// "No resident-byte accounting" section for why this is computed
/// rather than measured.
fn estimate_resident_bytes(dim: usize, nbits: u32, n_docs: usize) -> usize {
    let code_bytes_per_row = sub_quantizer_count(dim) * (nbits as usize).div_ceil(8);
    let codebook_bytes = CODEBOOK_CENTROIDS * dim * F32_BYTES;
    code_bytes_per_row * n_docs + codebook_bytes
}

pub struct FaissPqFastScanVectorEngine;
pub struct FaissPqVectorEngine;

pub struct FaissPqVectorIndex {
    // `RefCell` so `VectorEngine::read` / `VectorRead::topk_global`
    // (both `&self`) can reach `Index::search`, which FAISS's binding
    // exposes as `&mut self` — see the module doc-comment.
    index: RefCell<Option<IndexImpl>>,
    dim: usize,
    metric: VectorMetric,
    /// `PQ<M>x4fs` for the fast-scan engine, `PQ<M>x8` for the plain one.
    description: String,
}

/// Shared recall-calibration hook, mirroring the Lance/TurboQuant
/// adapters. FAISS PQ has no `(probe, refine)` vocabulary — it is a
/// flat scan over every quantized code — so both knobs are ignored, the
/// same posture `sq4flat.rs` and `turboquant/vector.rs` take for their
/// own knob-less scanners.
impl VectorRead for FaissPqVectorIndex {
    fn topk_global(
        &self,
        _column: &str,
        query: &[f32],
        k: usize,
        _nprobe: usize,
        _rerank: usize,
    ) -> Vec<(u32, f32)> {
        read_index(self, query, k)
            .into_iter()
            .map(|h| (h.doc_id as u32, h.distance))
            .collect()
    }

    fn search_params(&self, nprobe: usize, rerank: usize) -> String {
        if nprobe == ENGINE_DEFAULT && rerank == ENGINE_DEFAULT {
            format!("{} scan (no knobs)", self.description)
        } else {
            format!("{} scan (p/r ignored)", self.description)
        }
    }
}

fn create_index(dim: usize, metric: VectorMetric, description: String) -> FaissPqVectorIndex {
    FaissPqVectorIndex {
        index: RefCell::new(None),
        dim,
        metric,
        description,
    }
}

/// Cosine support: copies and L2-normalizes `vectors` when the index
/// metric is `Cosine`; returns the input slice unchanged otherwise. The
/// `Vec` is threaded back out through `owned` so the borrow the caller
/// takes (`owned.as_slice()`) outlives this call.
fn maybe_normalize<'a>(metric: VectorMetric, vectors: &'a [f32], dim: usize, owned: &'a mut Vec<f32>) -> &'a [f32] {
    if matches!(metric, VectorMetric::Cosine) {
        *owned = vectors.to_vec();
        normalize_rows(owned, dim);
        owned.as_slice()
    } else {
        vectors
    }
}

fn build_index(description: &str, vectors: &[f32], dim: usize, metric: VectorMetric) -> IndexImpl {
    let mut owned = Vec::new();
    let training_vectors = maybe_normalize(metric, vectors, dim, &mut owned);
    let n_docs = training_vectors.len() / dim;

    let mut built =
        index_factory(dim as u32, description, map_metric(metric)).expect("faiss index_factory");
    built.train(training_vectors).expect("faiss PQ train");
    let ids: Vec<Idx> = (0..n_docs as u64).map(Idx::new).collect();
    built
        .add_with_ids(training_vectors, &ids)
        .expect("faiss add_with_ids");
    built
}

fn nbits_for(description: &str) -> u32 {
    if description.ends_with("fs") {
        FAST_SCAN_NBITS
    } else {
        PLAIN_PQ_NBITS
    }
}

fn write_index(index: &mut FaissPqVectorIndex, vectors: &[f32]) {
    let n_docs = (vectors.len() / index.dim).max(1);
    let built = build_index(&index.description, vectors, index.dim, index.metric);
    let nbits = nbits_for(&index.description);
    let estimated = estimate_resident_bytes(index.dim, nbits, n_docs);
    eprintln!(
        "[comparison-vector/faiss-{}] estimated resident = {} ({} B/vec, {nbits}-bit codes; \
         analytical estimate, not measured — see module docs)",
        index.description,
        fmt_bytes(estimated as u64),
        estimated / n_docs,
    );
    *index.index.borrow_mut() = Some(built);
}

fn parallel_write_index(
    vectors: &[f32],
    dim: usize,
    metric: VectorMetric,
    description: &str,
    writers: usize,
) {
    // Build throughput is not the axis under test here — mirrors
    // `sq4flat.rs`'s `parallel_write_index`, which builds once
    // regardless of `writers` rather than fabricating a parallel-build
    // story the FAISS C API does not expose a natural seam for
    // (`index_factory` + `train` + `add_with_ids` is a single-threaded
    // call sequence; FAISS's own internal OpenMP parallelism, not a
    // caller-controlled writer count, is what runs underneath it).
    let _ = writers;
    let built = build_index(description, vectors, dim, metric);
    std::hint::black_box(&built);
}

fn read_index(index: &FaissPqVectorIndex, query: &[f32], k: usize) -> Vec<VectorHit> {
    let mut owned = Vec::new();
    let query = maybe_normalize(index.metric, query, index.dim, &mut owned);

    let mut guard = index.index.borrow_mut();
    let faiss_index = guard.as_mut().expect("index requested before write");
    let result = faiss_index.search(query, k).expect("faiss search");

    let negate = matches!(index.metric, VectorMetric::NegDot);
    result
        .labels
        .into_iter()
        .zip(result.distances)
        .filter_map(|(id, distance)| {
            id.get().map(|doc_id| VectorHit {
                doc_id,
                // Inner-product metrics are higher-is-better in FAISS;
                // negate to match the trait's smaller-is-better contract
                // (the same NegDot normalization `turboquant/vector.rs`
                // applies for its own inner-product-native library).
                // Cosine also runs on `InnerProduct` under the hood (see
                // the module doc-comment's "Metric support" section)
                // but the trait's `Cosine` is smaller-is-better by
                // convention among the *other* adapters here (Lance
                // reports `_distance` directly for its `Cosine`
                // `DistanceType`), so only `NegDot` is negated.
                distance: if negate { -distance } else { distance },
            })
        })
        .collect()
}

const CAPABILITIES: Capabilities = Capabilities {
    fts: false,
    vector: true,
    sql: false,
    hybrid: false,
};

impl VectorEngine for FaissPqFastScanVectorEngine {
    type Index = FaissPqVectorIndex;

    fn name() -> &'static str {
        "faiss-pq-fastscan"
    }

    fn capabilities() -> Capabilities {
        CAPABILITIES
    }

    fn create(_column: &str, dim: usize, metric: VectorMetric) -> Self::Index {
        create_index(dim, metric, fast_scan_description(dim))
    }

    fn write(index: &mut Self::Index, vectors: &[f32]) {
        write_index(index, vectors);
    }

    fn parallel_write(
        _column: &str,
        vectors: &[f32],
        dim: usize,
        metric: VectorMetric,
        writers: usize,
    ) {
        parallel_write_index(vectors, dim, metric, &fast_scan_description(dim), writers);
    }

    fn read(index: &Self::Index, query: &[f32], k: usize, _search: VectorSearch) -> Vec<VectorHit> {
        read_index(index, query, k)
    }

    fn close(index: &mut Self::Index) {
        *index.index.borrow_mut() = None;
    }

    fn delete(index: Self::Index) {
        drop(index);
    }
}

impl VectorEngine for FaissPqVectorEngine {
    type Index = FaissPqVectorIndex;

    fn name() -> &'static str {
        "faiss-pq"
    }

    fn capabilities() -> Capabilities {
        CAPABILITIES
    }

    fn create(_column: &str, dim: usize, metric: VectorMetric) -> Self::Index {
        create_index(dim, metric, plain_pq_description(dim))
    }

    fn write(index: &mut Self::Index, vectors: &[f32]) {
        write_index(index, vectors);
    }

    fn parallel_write(
        _column: &str,
        vectors: &[f32],
        dim: usize,
        metric: VectorMetric,
        writers: usize,
    ) {
        parallel_write_index(vectors, dim, metric, &plain_pq_description(dim), writers);
    }

    fn read(index: &Self::Index, query: &[f32], k: usize, _search: VectorSearch) -> Vec<VectorHit> {
        read_index(index, query, k)
    }

    fn close(index: &mut Self::Index) {
        *index.index.borrow_mut() = None;
    }

    fn delete(index: Self::Index) {
        drop(index);
    }
}
