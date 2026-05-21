//! Superfile FTS bench (1M docs). Standalone binary comparing infino vs Tantivy.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench fts-superfile                            # run all superfile FTS
//! cargo bench --bench fts-superfile -- <filter>               # criterion regex filter
//! INFINO_BENCH_UPDATE_README=1 cargo bench --bench fts-superfile
//! ```

mod superfile;

criterion::criterion_main!(superfile::benches);
