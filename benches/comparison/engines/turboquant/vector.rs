// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! TurboQuant (`turbovec`) peer implementation of [`VectorEngine`].
//!
//! A compressed flat index: every vector is rotated, quantized to 2 or 4
//! bits per coordinate (Lloyd-Max codebook + per-vector length renorm),
//! and every query scans all N compressed codes with a SIMD kernel. There
//! is no routing structure and therefore no search-time knob: `nprobe` /
//! `rerank` are accepted and ignored, so the calibrated "bar" row
//! coincides with the default row by construction.
//!
//! Scoring is inner-product-native. The comparison cells run `Cosine`
//! over unit-norm embedding corpora, where inner product and cosine give
//! the same ranking; the adapter refuses `L2Sq` rather than silently
//! ranking by the wrong metric. Returned `distance` is the negated score
//! so smaller-is-better holds, matching the trait contract.
//!
//! The measured `search()` path and its internal threading are the
//! library's own (rayon inside the kernel; `RAYON_NUM_THREADS` governs) —
//! the adapter adds no batching, so a `k`-search here is the true
//! single-query (`nq = 1`) cost, not the published batch-amortized one.

use turbovec::TurboQuantIndex;

use infino_bench_utils::executors::vector::{ENGINE_DEFAULT, VectorRead};
use infino_bench_utils::harness::{
    Capabilities, VectorEngine, VectorHit, VectorMetric, VectorSearch,
};
use infino_bench_utils::rss::fmt_bytes;

/// The two bit widths the library ships; one adapter type per width so
/// the shared driver can enumerate them as distinct engines.
const BIT_WIDTH_4: usize = 4;
const BIT_WIDTH_2: usize = 2;

pub struct Turbovec4VectorEngine;
pub struct Turbovec2VectorEngine;

pub struct TurbovecVectorIndex {
    index: Option<TurboQuantIndex>,
    bit_width: usize,
}

impl TurbovecVectorIndex {
    fn index(&self) -> &TurboQuantIndex {
        self.index.as_ref().expect("index requested before write")
    }

    /// Exact serialized index size — the resident-bytes number the
    /// comparison reports beside RSS (codes + codebook + norms).
    pub fn index_bytes(&self) -> usize {
        self.index().serialized_len()
    }
}

/// Shared recall-calibration hook, mirroring the Lance adapter: ids are
/// insertion positions (`0..n_docs`), already global. The flat scan has
/// no `(probe, refine)` vocabulary, so both knobs are ignored.
impl VectorRead for TurbovecVectorIndex {
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
            format!("flat {}-bit scan (no knobs)", self.bit_width)
        } else {
            format!("flat {}-bit scan (p/r ignored)", self.bit_width)
        }
    }
}

fn assert_supported_metric(metric: VectorMetric) {
    assert!(
        !matches!(metric, VectorMetric::L2Sq),
        "turbovec scores inner product only; L2Sq ranking would be wrong \
         unless the corpus is unit-norm — refuse rather than mismeasure"
    );
}

fn create_index(dim: usize, metric: VectorMetric, bit_width: usize) -> TurbovecVectorIndex {
    assert_supported_metric(metric);
    TurbovecVectorIndex {
        index: Some(TurboQuantIndex::new(dim, bit_width).expect("construct TurboQuantIndex")),
        bit_width,
    }
}

fn write_index(index: &mut TurbovecVectorIndex, vectors: &[f32]) {
    let idx = index.index.as_mut().expect("write after delete");
    idx.add(vectors);
    // Seal the blocked scan layout inside the build phase, exactly as the
    // trait contract asks ("`write` performs the ingest *and* seals the
    // index"): otherwise the first measured query pays the packing cost.
    idx.prepare();
    eprintln!(
        "[comparison-vector/turbovec-{}bit] index bytes = {} ({} B/vec)",
        index.bit_width,
        fmt_bytes(idx.serialized_len() as u64),
        idx.serialized_len() / (idx.len().max(1)),
    );
}

fn parallel_write_index(
    vectors: &[f32],
    dim: usize,
    metric: VectorMetric,
    writers: usize,
    bit_width: usize,
) {
    assert_supported_metric(metric);
    if writers <= 1 {
        let mut idx = TurboQuantIndex::new(dim, bit_width).expect("construct TurboQuantIndex");
        idx.add(vectors);
        idx.prepare();
        std::hint::black_box(&idx);
        return;
    }
    // Concurrent shard builds — the same independent-shard semantics as
    // the Infino and Lance parallel builds. Build-only; shards discarded.
    let n_docs = vectors.len() / dim;
    let docs_per_shard = n_docs.div_ceil(writers);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..writers)
            .filter_map(|shard| {
                let start_doc = shard * docs_per_shard;
                if start_doc >= n_docs {
                    return None;
                }
                let len_docs = docs_per_shard.min(n_docs - start_doc);
                let slice = &vectors[start_doc * dim..(start_doc + len_docs) * dim];
                Some(scope.spawn(move || {
                    let mut idx = TurboQuantIndex::new(dim, bit_width)
                        .expect("construct TurboQuantIndex shard");
                    idx.add(slice);
                    idx.prepare();
                    std::hint::black_box(&idx);
                }))
            })
            .collect();
        for h in handles {
            h.join().expect("turbovec shard build thread panicked");
        }
    });
}

fn read_index(index: &TurbovecVectorIndex, query: &[f32], k: usize) -> Vec<VectorHit> {
    let results = index.index().search(query, k);
    let ids = results.indices_for_query(0);
    let scores = results.scores_for_query(0);
    ids.iter()
        .zip(scores)
        .filter(|(id, _)| **id >= 0)
        .map(|(id, score)| VectorHit {
            doc_id: *id as u64,
            // Inner product, higher-is-better → negate for the trait's
            // smaller-is-better contract (the NegDot normalization).
            distance: -*score,
        })
        .collect()
}

/// Native `TurboQuantIndex::add` / `swap_remove` / `to_bytes` /
/// `from_bytes`. Ids are insertion positions (`0..len`); `insert` only
/// appends at `next_id == len`.
fn insert_index(index: &mut TurbovecVectorIndex, vectors: &[f32], next_id: u64) -> bool {
    let idx = index.index.as_mut().expect("insert after delete");
    assert_eq!(
        next_id,
        idx.len() as u64,
        "turbovec insert appends at the current length; ids are insertion positions"
    );
    idx.add(vectors);
    idx.prepare();
    true
}

fn remove_index(index: &mut TurbovecVectorIndex, ids: &[u64]) -> bool {
    let idx = index.index.as_mut().expect("remove after delete");
    // High-to-low so each `swap_remove` leaves the remaining named slots
    // valid. Removing the last `n` rows is a pure pop each time.
    let mut slots: Vec<usize> = ids.iter().map(|&id| id as usize).collect();
    slots.sort_unstable();
    slots.reverse();
    for slot in slots {
        idx.swap_remove(slot);
    }
    idx.prepare();
    true
}

fn save_index(index: &TurbovecVectorIndex) -> Option<Vec<u8>> {
    Some(index.index().to_bytes())
}

fn load_index(
    dim: usize,
    metric: VectorMetric,
    bytes: &[u8],
    bit_width: usize,
) -> Option<TurbovecVectorIndex> {
    assert_supported_metric(metric);
    let idx = TurboQuantIndex::from_bytes(bytes).expect("TurboQuantIndex::from_bytes");
    assert_eq!(
        idx.dim_opt(),
        Some(dim),
        "loaded turbovec dim must match the cell"
    );
    idx.prepare();
    Some(TurbovecVectorIndex {
        index: Some(idx),
        bit_width,
    })
}

/// Both widths share every code path; only the constructor's bit width
/// and the display name differ, exactly like the Lance local/S3 pair.
const CAPABILITIES: Capabilities = Capabilities {
    fts: false,
    vector: true,
    sql: false,
    hybrid: false,
    vector_insert: true,
    vector_remove: true,
    vector_save_load: true,
};

impl VectorEngine for Turbovec4VectorEngine {
    type Index = TurbovecVectorIndex;

    fn name() -> &'static str {
        "turbovec-4bit"
    }

    fn capabilities() -> Capabilities {
        CAPABILITIES
    }

    fn create(_column: &str, dim: usize, metric: VectorMetric) -> Self::Index {
        create_index(dim, metric, BIT_WIDTH_4)
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
        parallel_write_index(vectors, dim, metric, writers, BIT_WIDTH_4);
    }

    fn read(index: &Self::Index, query: &[f32], k: usize, _search: VectorSearch) -> Vec<VectorHit> {
        read_index(index, query, k)
    }

    fn close(_index: &mut Self::Index) {}

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
        load_index(dim, metric, bytes, BIT_WIDTH_4)
    }
}

impl VectorEngine for Turbovec2VectorEngine {
    type Index = TurbovecVectorIndex;

    fn name() -> &'static str {
        "turbovec-2bit"
    }

    fn capabilities() -> Capabilities {
        CAPABILITIES
    }

    fn create(_column: &str, dim: usize, metric: VectorMetric) -> Self::Index {
        create_index(dim, metric, BIT_WIDTH_2)
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
        parallel_write_index(vectors, dim, metric, writers, BIT_WIDTH_2);
    }

    fn read(index: &Self::Index, query: &[f32], k: usize, _search: VectorSearch) -> Vec<VectorHit> {
        read_index(index, query, k)
    }

    fn close(_index: &mut Self::Index) {}

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
        load_index(dim, metric, bytes, BIT_WIDTH_2)
    }
}
