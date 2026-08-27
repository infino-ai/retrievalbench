// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Infino's 4-bit codec as a **terminal flat scan** — the configuration
//! that competes with a compressed flat index on its own terms.
//!
//! This is not a shipping infino serving mode. It drives the engine's
//! real 4-bit encoder and SIMD nibble kernel through the
//! `test_helpers::sq4_flat` seam, scanning every vector per query, so
//! that the recall it reports is bounded by quantization error alone —
//! the same thing a peer compressed flat index is bounded by. The graph
//! arms cannot answer that question, because a walk's recall folds in
//! routing error and its bytes carry adjacency the scan does not need.
//!
//! Two widths are exposed, matching the peer's two: the bare 0.5
//! byte/dim plane, and the 1 byte/dim coarse-plus-residual construction.

use infino::test_helpers::sq4_flat::Sq4FlatIndex;

use infino_bench_utils::executors::vector::VectorRead;
use infino_bench_utils::harness::{
    Capabilities, VectorEngine, VectorHit, VectorMetric, VectorSearch,
};
use infino_bench_utils::rss::fmt_bytes;

/// Rotation seed for the probe plane. Fixed so a run is reproducible;
/// any seeded orthogonal rotation isotropizes the coordinates equally,
/// so the value carries no tuning.
const PROBE_ROT_SEED: u64 = 0x5147_5240_7031_A11E;

pub struct Sq4FlatVectorEngine;
pub struct Sq4ResidualFlatVectorEngine;

pub struct Sq4FlatVectorIndex {
    index: Option<Sq4FlatIndex>,
    dim: usize,
    with_residual: bool,
}

impl Sq4FlatVectorIndex {
    fn index(&self) -> &Sq4FlatIndex {
        self.index.as_ref().expect("index requested before write")
    }

    /// Bytes held resident to serve, as stored today (power-of-two
    /// rotation padding included).
    pub fn resident_bytes(&self) -> usize {
        self.index().resident_bytes()
    }

    /// Independently recomputed byte floor; equal to
    /// [`Self::resident_bytes`] unless padding creeps back into the plane.
    pub fn minimum_bytes(&self) -> usize {
        self.index().minimum_bytes()
    }
}

/// Per-`k` recall hook, matching the other adapters. A flat scan has no
/// `(probe, refine)` vocabulary, so both knobs are ignored.
impl VectorRead for Sq4FlatVectorIndex {
    fn topk_global(
        &self,
        _column: &str,
        query: &[f32],
        k: usize,
        _nprobe: usize,
        _rerank: usize,
    ) -> Vec<(u32, f32)> {
        self.index().search(query, k)
    }

    fn search_params(&self, _nprobe: usize, _rerank: usize) -> String {
        if self.with_residual {
            "flat 4-bit + residual scan (no knobs)".into()
        } else {
            "flat 4-bit scan (no knobs)".into()
        }
    }
}

fn assert_supported_metric(metric: VectorMetric) {
    assert!(
        !matches!(metric, VectorMetric::L2Sq),
        "the 4-bit plane scores NegDot; L2Sq ranking would be wrong unless \
         the corpus is unit-norm — refuse rather than mismeasure"
    );
}

fn create_index(dim: usize, metric: VectorMetric, with_residual: bool) -> Sq4FlatVectorIndex {
    assert_supported_metric(metric);
    Sq4FlatVectorIndex {
        index: None,
        dim,
        with_residual,
    }
}

fn write_index(index: &mut Sq4FlatVectorIndex, vectors: &[f32]) {
    let built = Sq4FlatIndex::build(vectors, index.dim, PROBE_ROT_SEED, index.with_residual);
    let label = if index.with_residual {
        "sq4-residual-flat"
    } else {
        "sq4-flat"
    };
    let rows = built.len().max(1);
    eprintln!(
        "[comparison-vector/{label}] resident = {} ({} B/vec); floor = {} ({} B/vec)",
        fmt_bytes(built.resident_bytes() as u64),
        built.resident_bytes() / rows,
        fmt_bytes(built.minimum_bytes() as u64),
        built.minimum_bytes() / rows,
    );
    index.index = Some(built);
}

fn parallel_write_index(vectors: &[f32], dim: usize, metric: VectorMetric, with_residual: bool) {
    assert_supported_metric(metric);
    // Build throughput is not the axis under test here; build once so the
    // driver's N-writer column has a defined value rather than a fabricated
    // parallel story the seam does not implement.
    let built = Sq4FlatIndex::build(vectors, dim, PROBE_ROT_SEED, with_residual);
    std::hint::black_box(&built);
}

const CAPABILITIES: Capabilities = Capabilities {
    fts: false,
    vector: true,
    sql: false,
    hybrid: false,
    // A flat scan plane is built once from the whole corpus; this
    // measurement seam exposes no add/remove path.
    vector_insert: false,
    vector_remove: false,
    vector_save_load: false,
};

impl VectorEngine for Sq4FlatVectorEngine {
    type Index = Sq4FlatVectorIndex;

    fn name() -> &'static str {
        "infino-sq4-flat"
    }

    fn capabilities() -> Capabilities {
        CAPABILITIES
    }

    fn create(_column: &str, dim: usize, metric: VectorMetric) -> Self::Index {
        create_index(dim, metric, false)
    }

    fn write(index: &mut Self::Index, vectors: &[f32]) {
        write_index(index, vectors);
    }

    fn parallel_write(
        _column: &str,
        vectors: &[f32],
        dim: usize,
        metric: VectorMetric,
        _writers: usize,
    ) {
        parallel_write_index(vectors, dim, metric, false);
    }

    fn read(index: &Self::Index, query: &[f32], k: usize, _search: VectorSearch) -> Vec<VectorHit> {
        index
            .index()
            .search(query, k)
            .into_iter()
            .map(|(node, score)| VectorHit {
                doc_id: node as u64,
                distance: score,
            })
            .collect()
    }

    fn close(_index: &mut Self::Index) {}

    fn delete(index: Self::Index) {
        drop(index);
    }
}

impl VectorEngine for Sq4ResidualFlatVectorEngine {
    type Index = Sq4FlatVectorIndex;

    fn name() -> &'static str {
        "infino-sq4res-flat"
    }

    fn capabilities() -> Capabilities {
        CAPABILITIES
    }

    fn create(_column: &str, dim: usize, metric: VectorMetric) -> Self::Index {
        create_index(dim, metric, true)
    }

    fn write(index: &mut Self::Index, vectors: &[f32]) {
        write_index(index, vectors);
    }

    fn parallel_write(
        _column: &str,
        vectors: &[f32],
        dim: usize,
        metric: VectorMetric,
        _writers: usize,
    ) {
        parallel_write_index(vectors, dim, metric, true);
    }

    fn read(index: &Self::Index, query: &[f32], k: usize, _search: VectorSearch) -> Vec<VectorHit> {
        index
            .index()
            .search(query, k)
            .into_iter()
            .map(|(node, score)| VectorHit {
                doc_id: node as u64,
                distance: score,
            })
            .collect()
    }

    fn close(_index: &mut Self::Index) {}

    fn delete(index: Self::Index) {
        drop(index);
    }
}
