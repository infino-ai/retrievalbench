//! Hybrid (FTS + vector) supertable bench bundle.
//!
//! Owns the realistic cross-topic supertable shape: a single
//! `Supertable` with **both** an FTS-indexed column (`title`) and a
//! vector-indexed column (`emb`) on the same docs. Build throughput
//! across writer-pool sizes, append latency, query latency for both
//! query types, plus the dual-pool reader-p99-under-writer-load
//! mixed-load epilogue.
//!
//! Per-topic supertable benches (`fts/supertable/`,
//! `vector/supertable/`) test single-topic supertables for tight
//! cross-engine isolation against Tantivy / LanceDB. This bundle
//! complements them with the full hybrid build + write-path
//! measurement that neither per-topic bench covers.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench hybrid                                # every group at default (1M-doc) scale
//! cargo bench --bench hybrid -- supertable_build            # only the build group
//! cargo bench --bench hybrid -- supertable_query            # only the query group
//! INFINO_BENCH_FULL=1 cargo bench --bench hybrid -- --quick # 10M-doc scale (point estimates)
//! ```

#[path = "../utils/corpus.rs"]
mod corpus;

#[path = "common.rs"]
mod hybrid_common;

#[path = "build.rs"]
mod build;
#[path = "append.rs"]
mod append;
#[path = "search.rs"]
mod search;
#[path = "mixed_load.rs"]
mod mixed_load;

criterion::criterion_main!(
    build::benches,
    append::benches,
    search::benches,
    mixed_load::benches,
);
