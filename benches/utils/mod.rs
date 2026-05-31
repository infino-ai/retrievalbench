//! Shared bench utilities for retrievalbench.

/// Criterion tree for the sibling infino repo (must match `Cargo.toml` `path` dep).
pub const INFINO_CRITERION_ROOT: &str = "../infino-pr8/target/criterion";

pub mod corpus;
pub mod lance;
pub mod markdown;
pub mod object_store_tier;
pub mod results;
pub mod rss;
