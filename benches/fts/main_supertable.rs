//! Supertable FTS bench (10M docs). Standalone binary comparing infino vs Tantivy.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench supertable_all -- supertable_fts          # run supertable FTS
//! cargo bench --bench supertable_all -- supertable_fts_search   # search only
//! INFINO_BENCH_UPDATE_README=1 cargo bench --bench supertable_all
//! ```

mod supertable;

criterion::criterion_main!(supertable::benches);
