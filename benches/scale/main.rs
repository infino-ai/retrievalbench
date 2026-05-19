//! Scale bench bundle. Mix of (a) phase-based stress runners that
//! exercise the writer + manifest stack at extreme size and (b)
//! at-scale pinned-recall assertion runners that need release-profile
//! compilation to finish in seconds rather than minutes. Each `run()`
//! reads its own env-var knobs for scale selection and prints
//! single-line summaries per phase to stderr.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench scale                                     # run every scale bench at default smoke scale
//! cargo bench --bench scale -- 100gb_laptop                     # only the 100GB laptop runner
//! cargo bench --bench scale -- 1m_segments                      # only the 1M-segment manifest stress
//! cargo bench --bench scale -- fts_recall                       # 20-check FTS pinned-recall battery (20K-doc Zipf)
//! cargo bench --bench scale -- vector_recall                    # 4-check vector pinned-recall battery (10K × 384)
//! cargo bench --bench scale -- oracle_calibrated_recall          # supertable-vs-Lance recall + Jaccard parity (5K × 384, 500 queries)
//! INFINO_BENCH_100GB=1 cargo bench --bench scale -- 100gb_laptop          # 43M docs (~100 GB)
//! INFINO_BENCH_M14_N_DOCS=1000000 cargo bench --bench scale -- 100gb_laptop   # 1M-doc smoke
//! INFINO_BENCH_M15D_MEDIUM=1 cargo bench --bench scale -- 1m_segments        # 10K superfiles
//! INFINO_BENCH_M15D_FULL=1   cargo bench --bench scale -- 1m_segments        # 1M superfiles
//! ```

#[path = "../utils/corpus.rs"]
mod corpus;

#[path = "supertable_100gb_laptop.rs"]
mod supertable_100gb_laptop;
#[path = "supertable_1m_segments.rs"]
mod supertable_1m_segments;
#[path = "fts_recall.rs"]
mod fts_recall;
#[path = "vector_recall.rs"]
mod vector_recall;
#[path = "oracle_calibrated_recall_targets_match_lance.rs"]
mod oracle_calibrated_recall_targets_match_lance;
#[path = "supertable_ingest_once.rs"]
mod supertable_ingest_once;

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();
    let run_all = filter.is_empty();
    let want = |needle: &str| run_all || filter.contains(needle);

    if want("100gb") {
        eprintln!("[scale] --- supertable_100gb_laptop ---");
        supertable_100gb_laptop::run();
    }
    if want("1m") || filter.contains("superfiles") {
        eprintln!("[scale] --- supertable_1m_segments ---");
        supertable_1m_segments::run();
    }
    if want("fts_recall") {
        eprintln!("[scale] --- fts_recall ---");
        fts_recall::run();
    }
    if want("vector_recall") {
        eprintln!("[scale] --- vector_recall ---");
        vector_recall::run();
    }
    if want("oracle_calibrated_recall") {
        eprintln!("[scale] --- oracle_calibrated_recall_targets_match_lance ---");
        oracle_calibrated_recall_targets_match_lance::run();
    }
    if want("supertable_ingest_once") {
        eprintln!("[scale] --- supertable_ingest_once ---");
        supertable_ingest_once::run();
    }
}
