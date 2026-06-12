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

pub mod lance;
pub mod tantivy;

pub use lance::fts::{LanceFtsEngine, LanceRemoteFtsEngine};
pub use lance::sql::{LanceRemoteSqlEngine, LanceSqlEngine};
pub use lance::vector::{LanceRemoteVectorEngine, LanceVectorEngine};
pub use tantivy::fts::TantivyFtsEngine;
