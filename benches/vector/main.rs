//! Superfile vector bench bundle.
//!
//! Supertable vector comparisons live in `supertable_all`, alongside
//! the supertable FTS comparisons.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench superfile_vector
//! cargo bench --bench superfile_vector -- <filter>
//! ```

mod superfile;

criterion::criterion_main!(superfile::benches);
