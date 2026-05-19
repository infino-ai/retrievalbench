//! Independent data-structure microbenches.
//!
//! Each leaf measures a single primitive in isolation — no engine
//! pipeline, no head-to-head against another search engine. These
//! exist for codec-evolution work and load-bearing primitive
//! verification, separate from the full-pipeline benches under
//! `fts/`, `vector/`, `hybrid/`, and `scale/`.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench data_structures                # every primitive
//! cargo bench --bench data_structures -- bloom       # criterion regex filter
//! ```

#[path = "bloom.rs"]
mod bloom;

criterion::criterion_main!(bloom::benches);
