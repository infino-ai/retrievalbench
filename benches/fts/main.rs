//! FTS bench bundle. Wraps every full-text-search benchmark in
//! one criterion binary so the topic has a single `[[bench]]`
//! stanza in `Cargo.toml`.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench fts                                  # run every fts bench at default (1M-doc) scale
//! cargo bench --bench fts -- <filter>                      # criterion regex filter, e.g. "fts_search/single_rare"
//! INFINO_BENCH_FULL=1 cargo bench --bench fts -- --quick   # 10M-doc scale (point estimates)
//! ```

mod superfile;
mod supertable;

criterion::criterion_main!(superfile::benches, supertable::benches);
