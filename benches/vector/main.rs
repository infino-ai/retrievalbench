//! Vector bench bundle. Wraps every vector benchmark in one
//! criterion binary so the topic has a single `[[bench]]` stanza
//! in `Cargo.toml`.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench vector                                  # run every vector bench at default (1M-doc) scale
//! cargo bench --bench vector -- <filter>                      # criterion regex filter, e.g. "vector_search/infino_recall"
//! INFINO_BENCH_FULL=1 cargo bench --bench vector -- --quick   # 10M-doc scale (point estimates)
//! ```

#[path = "../utils/corpus.rs"]
mod corpus;
#[path = "../utils/lance.rs"]
mod lance;
#[path = "../utils/markdown.rs"]
mod markdown;

// Single-binary vector bench per topic.
#[path = "superfile.rs"]
mod superfile;
#[path = "supertable.rs"]
mod supertable;

criterion::criterion_main!(superfile::benches, supertable::benches);
