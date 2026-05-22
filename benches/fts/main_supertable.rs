//! Supertable FTS bench (10M docs). Standalone binary comparing infino vs Tantivy.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench fts-supertable                            # run all supertable FTS
//! cargo bench --bench fts-supertable -- <filter>               # criterion regex filter
//! INFINO_BENCH_FULL=1 cargo bench --bench fts-supertable        # alternative syntax for 10M-doc scale
//! INFINO_BENCH_UPDATE_README=1 cargo bench --bench fts-supertable
//! ```

mod supertable;

criterion::criterion_main!(supertable::benches);
