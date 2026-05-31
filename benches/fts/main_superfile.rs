//! Superfile FTS bench (1M docs). Standalone binary comparing infino vs Tantivy.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench superfile_fts                            # run all superfile FTS
//! cargo bench --bench superfile_fts -- <filter>                # criterion regex filter
//! INFINO_BENCH_UPDATE_README=1 cargo bench --bench superfile_fts
//! ```

mod superfile;

criterion::criterion_main!(superfile::benches);
