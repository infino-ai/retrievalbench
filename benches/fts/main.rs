//! Superfile FTS bench bundle.
//!
//! Supertable FTS comparisons live in `supertable_all`, alongside the
//! supertable vector comparisons.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench superfile_fts
//! cargo bench --bench superfile_fts -- <filter>
//! ```

mod superfile;

criterion::criterion_main!(superfile::benches);
