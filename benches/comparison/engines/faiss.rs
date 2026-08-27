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
//! FAISS PQ has no bits-per-dimension mode, so `M` is derived from the
//! target four-bit density: `M = d * 4 / nbits`. `PQ<d>x4fs` and
//! `PQ<d/2>x8` therefore both store four code bits per input dimension,
//! matching `Sq4FlatIndex` and `turbovec-4bit`. Both indexes are wrapped
//! in `IDMap` so `add_with_ids` and remove-by-id use stable ids rather
//! than FAISS's shifting positions.
//!
//! ## Metric support
//!
//! FAISS's `MetricType` has exactly two variants, `InnerProduct` and
//! `L2` — there is no native `Cosine`. The shared harness materializes
//! cosine corpora unit-normalized; this adapter normalizes queries and
//! inserted rows before handing them to an `InnerProduct` index, the
//! standard FAISS idiom for cosine ranking. `L2Sq` maps to FAISS `L2`,
//! and `NegDot` maps to
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
//! ## Serialized bytes
//!
//! `faiss-rs` exposes file-based `write_index` / `read_index`. The
//! adapter writes the canonical built index to a temporary file and
//! reports that exact byte length, matching turbovec's serialized-index
//! accounting and also exercising the save/load boundary.

use std::cell::RefCell;
use std::fs;
use std::io::Write;

use faiss::index::{IndexImpl, index_factory};
use faiss::selector::IdSelector;
use faiss::{
    Idx, Index, MetricType, read_index as read_faiss_index, write_index as write_faiss_index,
};
use tempfile::NamedTempFile;

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
/// Code density shared with Infino Sq4 and turbovec-4bit.
const TARGET_BITS_PER_DIM: usize = 4;
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

/// Sub-quantizer count for the requested code density. See
/// the module doc-comment's "Sizing `M`" section.
fn sub_quantizer_count(dim: usize, nbits: u32) -> usize {
    let bits = nbits as usize;
    assert!(
        (dim * TARGET_BITS_PER_DIM).is_multiple_of(bits),
        "FAISS dimension must divide the target code density"
    );
    dim * TARGET_BITS_PER_DIM / bits
}

/// Factory description string for the fast-scan variant: `PQ<M>x4fs`.
fn fast_scan_description(dim: usize) -> String {
    format!(
        "IDMap,PQ{}x{FAST_SCAN_NBITS}fs",
        sub_quantizer_count(dim, FAST_SCAN_NBITS)
    )
}

/// Factory description string for the plain LUT256 variant:
/// `PQ<M>x8np`. The `np` suffix skips training the Polysemous
/// permutation, which FAISS only uses for Hamming pre-filtering — a
/// mode this cell never queries — and whose training cost dominates
/// everything else at these sub-quantizer counts.
fn plain_pq_description(dim: usize) -> String {
    format!(
        "IDMap,PQ{}x{PLAIN_PQ_NBITS}np",
        sub_quantizer_count(dim, PLAIN_PQ_NBITS)
    )
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
    /// Exact serialized artifact size after the canonical build.
    serialized_bytes: usize,
}

impl FaissPqVectorIndex {
    pub fn serialized_bytes(&self) -> usize {
        self.serialized_bytes
    }
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
        serialized_bytes: 0,
    }
}

/// Cosine support: copies and L2-normalizes `vectors` when the index
/// metric is `Cosine`; returns the input slice unchanged otherwise. The
/// `Vec` is threaded back out through `owned` so the borrow the caller
/// takes (`owned.as_slice()`) outlives this call.
fn maybe_normalize<'a>(
    metric: VectorMetric,
    vectors: &'a [f32],
    dim: usize,
    owned: &'a mut Vec<f32>,
) -> &'a [f32] {
    if matches!(metric, VectorMetric::Cosine) {
        *owned = vectors.to_vec();
        normalize_rows(owned, dim);
        owned.as_slice()
    } else {
        vectors
    }
}

fn build_index(description: &str, vectors: &[f32], dim: usize, metric: VectorMetric) -> IndexImpl {
    // Every cosine corpus source in the shared harness is normalized while
    // it is materialized. Copying 10M × 768 fp32 values here solely to
    // normalize them again would add a second 30 GiB corpus allocation.
    if matches!(metric, VectorMetric::Cosine) {
        debug_assert!(vectors.chunks_exact(dim).take(16).all(|row| {
            let norm_sq = row.iter().map(|x| x * x).sum::<f32>();
            (norm_sq - 1.0).abs() < 0.01
        }));
    }
    let training_vectors = vectors;
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

fn save_native(index: &IndexImpl) -> Vec<u8> {
    let file = NamedTempFile::new().expect("create temporary FAISS index file");
    let path = file
        .path()
        .to_str()
        .expect("temporary FAISS path is valid UTF-8");
    write_faiss_index(index, path).expect("write FAISS index");
    fs::read(file.path()).expect("read serialized FAISS index")
}

fn load_native(bytes: &[u8]) -> IndexImpl {
    let mut file = NamedTempFile::new().expect("create temporary FAISS index file");
    file.write_all(bytes)
        .expect("write serialized FAISS index bytes");
    file.flush().expect("flush serialized FAISS index bytes");
    let path = file
        .path()
        .to_str()
        .expect("temporary FAISS path is valid UTF-8");
    read_faiss_index(path).expect("read FAISS index")
}

fn write_index(index: &mut FaissPqVectorIndex, vectors: &[f32]) {
    let n_docs = (vectors.len() / index.dim).max(1);
    let built = build_index(&index.description, vectors, index.dim, index.metric);
    let serialized_bytes = save_native(&built).len();
    eprintln!(
        "[comparison-vector/faiss-{}] serialized = {} ({} B/vec)",
        index.description,
        fmt_bytes(serialized_bytes as u64),
        serialized_bytes / n_docs,
    );
    index.serialized_bytes = serialized_bytes;
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

    result
        .labels
        .into_iter()
        .zip(result.distances)
        .filter_map(|(id, distance)| {
            id.get().map(|doc_id| VectorHit {
                doc_id,
                distance: match index.metric {
                    VectorMetric::L2Sq => distance,
                    VectorMetric::Cosine => 1.0 - distance,
                    VectorMetric::NegDot => -distance,
                },
            })
        })
        .collect()
}

fn insert_index(index: &mut FaissPqVectorIndex, vectors: &[f32], next_id: u64) -> bool {
    let mut owned = Vec::new();
    let vectors = maybe_normalize(index.metric, vectors, index.dim, &mut owned);
    let n_docs = vectors.len() / index.dim;
    let ids: Vec<Idx> = (next_id..next_id + n_docs as u64).map(Idx::new).collect();
    index
        .index
        .borrow_mut()
        .as_mut()
        .expect("FAISS insert before write")
        .add_with_ids(vectors, &ids)
        .expect("FAISS add_with_ids");
    true
}

fn remove_index(index: &mut FaissPqVectorIndex, ids: &[u64]) -> bool {
    let ids: Vec<Idx> = ids.iter().copied().map(Idx::new).collect();
    let selector = IdSelector::batch(&ids).expect("FAISS ID selector");
    let removed = index
        .index
        .borrow_mut()
        .as_mut()
        .expect("FAISS remove before write")
        .remove_ids(&selector)
        .expect("FAISS remove_ids");
    assert_eq!(removed, ids.len(), "FAISS removed every requested id");
    true
}

fn save_index(index: &FaissPqVectorIndex) -> Option<Vec<u8>> {
    let guard = index.index.borrow();
    Some(save_native(
        guard.as_ref().expect("FAISS save before write"),
    ))
}

fn load_index(
    dim: usize,
    metric: VectorMetric,
    bytes: &[u8],
    description: String,
) -> Option<FaissPqVectorIndex> {
    let native = load_native(bytes);
    assert_eq!(native.d() as usize, dim, "loaded FAISS dimension");
    Some(FaissPqVectorIndex {
        index: RefCell::new(Some(native)),
        dim,
        metric,
        description,
        serialized_bytes: bytes.len(),
    })
}

const CAPABILITIES: Capabilities = Capabilities {
    fts: false,
    vector: true,
    sql: false,
    hybrid: false,
    vector_insert: true,
    vector_remove: true,
    vector_save_load: true,
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

    fn insert(index: &mut Self::Index, vectors: &[f32], next_id: u64) -> bool {
        insert_index(index, vectors, next_id)
    }

    fn remove(index: &mut Self::Index, ids: &[u64]) -> bool {
        remove_index(index, ids)
    }

    fn save(index: &Self::Index) -> Option<Vec<u8>> {
        save_index(index)
    }

    fn load(_column: &str, dim: usize, metric: VectorMetric, bytes: &[u8]) -> Option<Self::Index> {
        load_index(dim, metric, bytes, fast_scan_description(dim))
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

    fn insert(index: &mut Self::Index, vectors: &[f32], next_id: u64) -> bool {
        insert_index(index, vectors, next_id)
    }

    fn remove(index: &mut Self::Index, ids: &[u64]) -> bool {
        remove_index(index, ids)
    }

    fn save(index: &Self::Index) -> Option<Vec<u8>> {
        save_index(index)
    }

    fn load(_column: &str, dim: usize, metric: VectorMetric, bytes: &[u8]) -> Option<Self::Index> {
        load_index(dim, metric, bytes, plain_pq_description(dim))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DIM: usize = 4;
    const TEST_ROWS: usize = 512;
    const INSERT_ID: u64 = 10_000;

    fn test_vectors() -> Vec<f32> {
        let mut vectors = (0..TEST_ROWS * TEST_DIM)
            .map(|i| ((i * 17 % 101) as f32 - 50.0) / 50.0)
            .collect::<Vec<_>>();
        normalize_rows(&mut vectors, TEST_DIM);
        vectors
    }

    fn total(index: &FaissPqVectorIndex) -> u64 {
        index.index.borrow().as_ref().expect("test index").ntotal()
    }

    fn lifecycle(description: String) {
        let mut index = create_index(TEST_DIM, VectorMetric::Cosine, description.clone());
        write_index(&mut index, &test_vectors());
        assert_eq!(total(&index), TEST_ROWS as u64);

        let extra = [1.0, 0.0, 0.0, 0.0];
        assert!(insert_index(&mut index, &extra, INSERT_ID));
        assert_eq!(total(&index), TEST_ROWS as u64 + 1);
        assert!(remove_index(&mut index, &[INSERT_ID]));
        assert_eq!(total(&index), TEST_ROWS as u64);

        let bytes = save_index(&index).expect("save test FAISS index");
        let loaded = load_index(TEST_DIM, VectorMetric::Cosine, &bytes, description)
            .expect("load test FAISS index");
        assert_eq!(total(&loaded), TEST_ROWS as u64);
    }

    #[test]
    fn pq_fastscan_supports_the_published_lifecycle() {
        lifecycle(fast_scan_description(TEST_DIM));
    }

    #[test]
    fn pq_lut256_supports_the_published_lifecycle() {
        lifecycle(plain_pq_description(TEST_DIM));
    }

    #[test]
    fn full_dimension_factories_match_four_bits_per_dimension() {
        assert_eq!(fast_scan_description(1024), "IDMap,PQ1024x4fs");
        assert_eq!(plain_pq_description(1024), "IDMap,PQ512x8np");
        let fast = index_factory(1024, fast_scan_description(1024), MetricType::InnerProduct)
            .expect("full-dimension PQFastScan factory");
        let plain = index_factory(1024, plain_pq_description(1024), MetricType::InnerProduct)
            .expect("full-dimension PQ factory");
        assert_eq!(fast.d(), 1024);
        assert_eq!(plain.d(), 1024);
    }
}
