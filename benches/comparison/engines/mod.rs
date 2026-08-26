// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Peer-engine adapters for the head-to-head comparison.
//!
//! Each adapter implements the `infino-bench-utils` engine traits
//! (`FtsEngine` / `VectorEngine` / `SqlEngine`) so the shared drivers can
//! measure it through the exact path they measure Infino. The Infino
//! sides of every comparison come from `infino-bench-utils` itself
//! (`InfinoFtsEngine` / `InfinoVectorEngine` / `InfinoSqlEngine`) and are
//! not re-declared here.

//! The FAISS peer is behind the `faiss` feature because it links against
//! a system `libfaiss` C++ build; see `Cargo.toml`.

#[cfg(feature = "faiss")]
pub mod faiss;
pub mod lance;
pub mod sq4flat;
pub mod tantivy;
pub mod turboquant;

#[cfg(feature = "faiss")]
pub use faiss::{FaissPqFastScanVectorEngine, FaissPqVectorEngine};
pub use lance::fts::{LanceFtsEngine, LanceS3FtsEngine};
pub use lance::location::lance_peer_label;
pub use lance::sql::{LanceS3SqlEngine, LanceSqlEngine};
pub use lance::vector::{LanceS3VectorEngine, LanceVectorEngine};
pub use sq4flat::{Sq4FlatVectorEngine, Sq4ResidualFlatVectorEngine};
pub use tantivy::fts::TantivyFtsEngine;
pub use turboquant::vector::{Turbovec2VectorEngine, Turbovec4VectorEngine};
