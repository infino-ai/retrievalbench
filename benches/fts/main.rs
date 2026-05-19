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

#[path = "../utils/corpus.rs"]
mod corpus;

// Shared markdown summary emitter — reads criterion's
// estimates.json after timing finishes and prints a markdown
// block to stderr (and, with `INFINO_BENCH_UPDATE_README=1`,
// rewrites the matching section of `benches/README.md`).
#[path = "../utils/markdown.rs"]
mod markdown;

// Single-binary FTS bench per topic. Each topic's file holds its
// ingest sub-group + search sub-group(s) + correctness gates +
// fixtures + builders.
#[path = "superfile.rs"]
mod superfile;
#[path = "supertable.rs"]
mod supertable;

criterion::criterion_main!(superfile::benches, supertable::benches);
