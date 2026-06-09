// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Peer-engine adapters for the head-to-head comparison.
//!
//! Each adapter implements an `infino-bench-utils` engine trait
//! (`FtsEngine` / `VectorEngine` / `SqlEngine`) so infino's own drivers
//! can measure the peer through the exact path they measure Infino.

mod lance_fts_engine;
mod lance_sql_engine;
mod lance_vector_engine;
mod tantivy_engine;

pub use lance_fts_engine::{LanceFtsEngine, LanceS3FtsEngine};
pub use lance_sql_engine::LanceSqlEngine;
pub use lance_vector_engine::LanceVectorEngine;
pub use tantivy_fts_engine::TantivyFtsEngine;
