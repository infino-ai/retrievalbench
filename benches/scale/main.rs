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
//! cargo bench --features bench-diagnostics --bench scale
//! cargo bench --features bench-diagnostics --bench scale -- 100gb_laptop
//! cargo bench --features bench-diagnostics --bench scale -- 1m_segments
//! cargo bench --features bench-diagnostics --bench scale -- fts_recall
//! cargo bench --features bench-diagnostics --bench scale -- vector_recall
//! cargo bench --features bench-diagnostics --bench scale -- oracle_calibrated_recall
//! INFINO_BENCH_100GB=1 cargo bench --features bench-diagnostics --bench scale -- 100gb_laptop
//! INFINO_BENCH_M14_N_DOCS=1000000 cargo bench --features bench-diagnostics --bench scale -- 100gb_laptop
//! INFINO_BENCH_M15D_MEDIUM=1 cargo bench --features bench-diagnostics --bench scale -- 1m_segments
//! INFINO_BENCH_M15D_FULL=1 cargo bench --features bench-diagnostics --bench scale -- 1m_segments
//! ```

mod fts_recall;
mod oracle_calibrated_recall_targets_match_lance;
mod supertable_100gb_laptop;
mod supertable_1m_segments;
mod supertable_ingest_once;
mod vector_recall;

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
