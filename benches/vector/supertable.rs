//! Lance head-to-head vector bench for the supertable layer.
//!
//! Measures Lance only — infino's own numbers come from `infino/benches`
//! (read out of `../infino/target/criterion/...` at emit time). The
//! corpus is shared with infino's bench via
//! `infino::test_helpers::bench_corpus`.
//!
//! 10M × 384-dim Gaussian planted-cluster corpus, normalized for cosine.
//! This is the VECTOR-ONLY comparison: Lance ingest builds a vector index
//! only (no FTS), apples-to-apples against infino's vector-only supertable
//! (`supertable_vec_build`). The FTS-only comparison vs Tantivy lives in
//! `benches/fts/supertable.rs`.
//! Lance's `num_partitions = N_CENT_TOTAL` is set to match the
//! aggregate cluster count that infino's supertable shards (the
//! supertable's per-segment IVF centroids sum to the same total).
//!
//! ## Workflow
//!
//! ```text
//! (cd ../infino-pr8 && cargo bench --bench supertable_all -- supertable_vec_build)   # populate infino vector-only ingest
//! cargo bench --bench supertable_all -- supertable_vec_build                         # measure Lance ingest + emit
//! ```

use std::collections::HashMap;
use std::hint::black_box;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group};
use lancedb::Table;
use retrievalbench::rss;
use tempfile::TempDir;
use tokio::runtime::Runtime;

use retrievalbench::corpus::{self, Calibrated, DIM};
use retrievalbench::lance;
use retrievalbench::markdown;
use retrievalbench::object_store_tier::{self, Tier};
use retrievalbench::results;

// ─── Constants ────────────────────────────────────────────────────────

const N_DOCS: usize = corpus::SUPERTABLE_DOCS;
const TOP_K: usize = 10;

const N_CORRECTNESS_QUERIES: usize = 20;
const N_CALIBRATION_QUERIES: usize = 100;

const RECALL_TARGETS: &[f32] = &[0.90, 0.95, 0.99];

const LANCE_PROBES: &[usize] = &[1, 5, 10, 25, 50, 100, 200, 400, 800];
const LANCE_REFINES: &[u32] = &[1, 4, 16, 64, 256, 1024];

const CORRECTNESS_RECALL_FLOOR: f32 = 0.80;
const CORRECTNESS_LANCE_PROBES: usize = 64;
const CORRECTNESS_LANCE_REFINE: u32 = 256;

// ─── Fixtures ────────────────────────────────────────────────────────

static VECTORS: OnceLock<Vec<f32>> = OnceLock::new();
static QUERIES_CORRECTNESS: OnceLock<Vec<Vec<f32>>> = OnceLock::new();
static QUERIES_CALIBRATION: OnceLock<Vec<Vec<f32>>> = OnceLock::new();
static GROUND_TRUTH_CORRECTNESS: OnceLock<Vec<Vec<u32>>> = OnceLock::new();
static GROUND_TRUTH_CALIBRATION: OnceLock<Vec<Vec<u32>>> = OnceLock::new();
static LANCE: OnceLock<LanceHandles> = OnceLock::new();
static CALIBRATIONS: OnceLock<Calibrations> = OnceLock::new();

struct S3LanceCommitted {
    uri: String,
    storage_options: HashMap<String, String>,
    storage_label: &'static str,
}
static S3_LANCE: OnceLock<S3LanceCommitted> = OnceLock::new();

fn vectors() -> &'static [f32] {
    VECTORS.get_or_init(|| corpus::generate_vector_corpus(N_DOCS, corpus::n_cent(N_DOCS), 1, true))
}

fn queries_correctness() -> &'static [Vec<f32>] {
    QUERIES_CORRECTNESS.get_or_init(|| {
        corpus::generate_realistic_queries(vectors(), N_DOCS, N_CORRECTNESS_QUERIES, 17, true, 0.05)
    })
}

fn queries_calibration() -> &'static [Vec<f32>] {
    QUERIES_CALIBRATION.get_or_init(|| {
        corpus::generate_realistic_queries(vectors(), N_DOCS, N_CALIBRATION_QUERIES, 99, true, 0.05)
    })
}

fn ground_truth_correctness() -> &'static [Vec<u32>] {
    GROUND_TRUTH_CORRECTNESS
        .get_or_init(|| corpus::ground_truth(vectors(), N_DOCS, queries_correctness(), TOP_K))
}

fn ground_truth_calibration() -> &'static [Vec<u32>] {
    GROUND_TRUTH_CALIBRATION
        .get_or_init(|| corpus::ground_truth(vectors(), N_DOCS, queries_calibration(), TOP_K))
}

fn lance_handles() -> &'static LanceHandles {
    LANCE.get_or_init(build_lance_handles)
}

fn s3_lance_committed() -> &'static S3LanceCommitted {
    S3_LANCE.get_or_init(|| {
        eprintln!(
            "[supertable_vec] committing Lance vector-only table ({N_DOCS} docs) to object storage for warm/cold tiers..."
        );
        let fixture = object_store_tier::block_on(object_store_tier::lance_storage_fixture());
        let rt = Runtime::new().expect("tokio runtime");
        let n_cent = corpus::n_cent(N_DOCS) as u32;
        let elapsed = lance::build_lance_table_uri(
            &rt,
            &fixture.lance_uri,
            &fixture.storage_options,
            vectors(),
            N_DOCS,
            n_cent,
            lance::default_n_sub_vectors(),
        );
        eprintln!(
            "[supertable_vec] Lance vector-only object-store commit OK in {:.1}s ({})",
            elapsed.as_secs_f64(),
            fixture.storage_label
        );
        S3LanceCommitted {
            uri: fixture.lance_uri,
            storage_options: fixture.storage_options,
            storage_label: fixture.storage_label,
        }
    })
}

// ─── Lance builder ────────────────────────────────────────────────────

struct LanceHandles {
    table: Table,
    _dir: TempDir,
    rt: Runtime,
    build_elapsed: Duration,
}

fn build_lance_handles() -> LanceHandles {
    let n_cent = corpus::n_cent(N_DOCS) as u32;
    let rt = Runtime::new().expect("tokio runtime");
    let dir = TempDir::new().expect("tempdir");
    eprintln!(
        "[supertable_vec] initializing shared vector-only Lance table ({N_DOCS} docs)..."
    );
    let (table, build_elapsed) = lance::build_lance_table(
        &rt,
        dir.path(),
        vectors(),
        N_DOCS,
        n_cent,
        lance::default_n_sub_vectors(),
    );
    LanceHandles {
        table,
        _dir: dir,
        rt,
        build_elapsed,
    }
}

// ─── Correctness ──────────────────────────────────────────────────────

fn assert_lance_self_consistent(lh: &LanceHandles) -> f32 {
    let mean_recall = lance::mean_recall_lance(
        &lh.rt,
        &lh.table,
        queries_correctness(),
        ground_truth_correctness(),
        TOP_K,
        CORRECTNESS_LANCE_PROBES,
        CORRECTNESS_LANCE_REFINE,
    );
    assert!(
        mean_recall >= CORRECTNESS_RECALL_FLOOR,
        "Lance mean recall@{TOP_K} at correctness config \
         (p={CORRECTNESS_LANCE_PROBES}, r={CORRECTNESS_LANCE_REFINE}) \
         below floor: {mean_recall:.3} < {CORRECTNESS_RECALL_FLOOR:.3}"
    );
    mean_recall
}

// ─── Calibration ──────────────────────────────────────────────────────

struct Calibrations {
    lance: [Option<Calibrated>; 3],
}

fn calibrations() -> &'static Calibrations {
    CALIBRATIONS.get_or_init(|| {
        let lh = lance_handles();
        let qs = queries_calibration();
        let gt = ground_truth_calibration();

        eprintln!(
            "[supertable_vec_search] calibrating Lance at recall targets {RECALL_TARGETS:?}..."
        );
        let mut l: [Option<Calibrated>; 3] = [None; 3];
        for (i, &target) in RECALL_TARGETS.iter().enumerate() {
            l[i] = lance::calibrate_lance(
                &lh.rt,
                &lh.table,
                qs,
                gt,
                target,
                LANCE_PROBES,
                LANCE_REFINES,
                21,
                TOP_K,
            );
            eprintln!("  recall ≥ {target:.2} | lance: {:?}", l[i]);
        }
        Calibrations { lance: l }
    })
}

// ─── Bench entry ──────────────────────────────────────────────────────

fn bench(c: &mut Criterion) {
    // ---- Ingest sub-bench (group: supertable_vec_build) ------------
    {
        let v = vectors();
        let n_cent = corpus::n_cent(N_DOCS) as u32;

        let mut g = c.benchmark_group(group_name::SUPERTABLE_VEC_BUILD);
        g.sample_size(10);
        g.throughput(Throughput::Elements(N_DOCS as u64));
        let rss_sample = rss::PeakSampler::start_default();

        g.bench_function(format!("lance_vec_{N_DOCS}docs"), |b| {
            b.iter_custom(|iters| {
                black_box(v);
                black_box(n_cent);
                let lh = lance_handles();
                lh.build_elapsed * (iters as u32)
            });
        });

        g.finish();
        let stats = rss_sample.stop_stats();
        let _ = rss::write_rss_stats(
            group_name::SUPERTABLE_VEC_BUILD,
            &format!("lance_vec_{N_DOCS}docs"),
            stats,
        );

        emit_ingest_markdown();
        emit_json_results();
    }

    eprintln!(
        "[supertable_vec] correctness: using shared vector-only Lance table ({N_DOCS} docs)..."
    );
    let lh = lance_handles();
    let recall = assert_lance_self_consistent(lh);
    eprintln!(
        "[supertable_vec] correctness OK: Lance vector recall@{TOP_K} = {recall:.3} (≥ {CORRECTNESS_RECALL_FLOOR:.2})"
    );

    // ---- Search sub-bench (group: supertable_vec_search) -----------
    {
        let cal = calibrations();
        let qs = queries_calibration();

        let mut g = c.benchmark_group("supertable_vec_hot_search");
        g.sample_size(10);
        let rss_sample = rss::PeakSampler::start_default();

        for (i, &target) in RECALL_TARGETS.iter().enumerate() {
            let label = format!("recall_at_least_{:02}", (target * 100.0) as u32);
            if let Some(c_la) = cal.lance[i] {
                let r_u32 = c_la.refine as u32;
                g.bench_with_input(
                    BenchmarkId::new(
                        format!("lance_{label}"),
                        format!("p={},r={}", c_la.probe, r_u32),
                    ),
                    &(c_la.probe, r_u32),
                    |b, &(p, r)| {
                        let q = &qs[0];
                        b.iter(|| {
                            let hits = lance::search_lance(&lh.rt, &lh.table, q, TOP_K, p, r);
                            black_box(hits)
                        });
                    },
                );
            }
        }

        g.finish();
        let stats = rss_sample.stop_stats();
        for (i, &target) in RECALL_TARGETS.iter().enumerate() {
            let label = format!("recall_at_least_{:02}", (target * 100.0) as u32);
            if let Some(c_la) = cal.lance[i] {
                let bid = format!("lance_{label}/p={},r={}", c_la.probe, c_la.refine as u32);
                let _ = rss::write_rss_stats(group_name::SUPERTABLE_VEC_SEARCH, &bid, stats);
            }
        }

        bench_search_lance_object_store_tiers(c, &cal, qs);

        emit_search_markdown();
        emit_json_results();
    }
}

fn bench_search_lance_object_store_tiers(c: &mut Criterion, cal: &Calibrations, qs: &[Vec<f32>]) {
    let committed = s3_lance_committed();
    let rt = Runtime::new().expect("tokio runtime");
    let q = &qs[0];

    for tier in [Tier::Warm, Tier::Cold] {
        let mut g = c.benchmark_group(format!(
            "supertable_vec_{}_search_lance_{}",
            tier.label(),
            committed.storage_label
        ));
        g.sample_size(10);

        for (i, &target) in RECALL_TARGETS.iter().enumerate() {
            let Some(c_la) = cal.lance[i] else {
                continue;
            };
            let label = format!("recall_at_least_{:02}", (target * 100.0) as u32);
            let (p, r) = (c_la.probe, c_la.refine as u32);
            let bench_id = format!("lance_{label}/p={p},r={r}");

            match tier {
                Tier::Warm => {
                    let table = lance::open_lance_vector_table(
                        &rt,
                        &committed.uri,
                        &committed.storage_options,
                    );
                    let query = q.clone();
                    let _ = lance::search_lance(&rt, &table, &query, TOP_K, p, r);
                    object_store_tier::block_on(object_store_tier::wait_after_prewarm());
                    g.bench_function(&bench_id, |b| {
                        let query = q.clone();
                        b.iter(|| {
                            let hits =
                                lance::search_lance(&rt, &table, black_box(&query), TOP_K, p, r);
                            black_box(hits)
                        });
                    });
                }
                Tier::Cold => {
                    let uri = committed.uri.clone();
                    let opts = committed.storage_options.clone();
                    let query = q.clone();
                    g.bench_function(&bench_id, |b| {
                        b.iter_custom(|iters| {
                            let mut total = Duration::ZERO;
                            for _ in 0..iters {
                                // Mirror infino's cold tier: opening the table loads
                                // index metadata once (amortized over the reader's life
                                // in production), so it is excluded from the per-query
                                // cold measurement. A fresh open per iteration keeps the
                                // data pages cold.
                                let table = lance::open_lance_vector_table(&rt, &uri, &opts);
                                let t0 = Instant::now();
                                let _ = lance::search_lance(&rt, &table, &query, TOP_K, p, r);
                                total += t0.elapsed();
                            }
                            total
                        });
                    });
                }
                Tier::Hot => {}
            }
        }
        g.finish();
    }
}

// ─── JSON results emitter ─────────────────────────────────────────────

fn emit_json_results() {
    let mut collector = results::ResultsCollector::new();

    // Collect build benchmark results
    collector.add_from_criterion(
        group_name::SUPERTABLE_VEC_BUILD,
        &format!("lance_vec_{N_DOCS}docs"),
        Some("lance"),
    );
    collector.add_from_infino(
        group_name::SUPERTABLE_VEC_BUILD,
        &format!("supertable_vec_{N_DOCS}docs"),
        Some("infino"),
    );

    // Collect search benchmark results
    let cal = calibrations();
    for (i, &target) in RECALL_TARGETS.iter().enumerate() {
        let label = format!("recall_at_least_{:02}", (target * 100.0) as u32);
        if let Some(c_la) = cal.lance[i] {
            let bid = format!("lance_{label}/p={},r={}", c_la.probe, c_la.refine as u32);
            collector.add_from_criterion_with_group(
                group_name::SUPERTABLE_VEC_SEARCH,
                &format!("supertable_vec_search_{label}"),
                &bid,
                Some("lance"),
            );
        }
        // Note: infino search results are stored as calibrated values and would require
        // more complex extraction. For now, we collect the main Lance results.
    }

    if let Err(e) = collector.emit() {
        eprintln!("[results] failed to emit JSON results: {e}");
    }
}

// ─── Markdown summary emitters ────────────────────────────────────────

mod group_name {
    pub const SUPERTABLE_VEC_BUILD: &str = "supertable_vec_build";
    pub const SUPERTABLE_VEC_SEARCH: &str = "supertable_vec_hot_search";
}

fn emit_ingest_markdown() {
    use markdown::{
        MarkdownSection, fmt_throughput, fmt_time, fmt_winner, read_infino_mean_ns, read_mean_ns,
    };

    let group = group_name::SUPERTABLE_VEC_BUILD;
    let infino_bench = format!("supertable_vec_{N_DOCS}docs");
    let lance_bench = format!("lance_vec_{N_DOCS}docs");
    let infino_ns = read_infino_mean_ns(group, &infino_bench);
    let lance_ns = read_mean_ns(group, &lance_bench);

    let mut body = String::new();
    body.push_str(&format!(
        "### Supertable vector-only — ingest ({N_DOCS} docs × dim={DIM})\n\n"
    ));
    body.push_str(
        "Both engines build one table with a vector index only (no FTS) before timing stops.\n\n",
    );
    body.push_str(
        "| Engine | Time | Throughput | Peak RSS | Median RSS | P90 RSS | Peak RSS Δ | vs LanceDB |\n",
    );
    body.push_str(
        "|--------|------|------------|----------|------------|---------|------------|------------|\n",
    );
    for (label, ns, peak_rss, median_rss, p90_rss, rss_delta, is_baseline) in [
        (
            "supertable",
            infino_ns,
            rss::read_infino_peak_rss_bytes(group, &infino_bench),
            rss::fmt_infino_median_rss(group, &infino_bench),
            rss::fmt_infino_p90_rss(group, &infino_bench),
            rss::fmt_infino_peak_rss_delta(group, &infino_bench),
            false,
        ),
        (
            "lance",
            lance_ns,
            rss::read_peak_rss_bytes(group, &lance_bench),
            rss::fmt_median_rss(group, &lance_bench),
            rss::fmt_p90_rss(group, &lance_bench),
            rss::fmt_peak_rss_delta(group, &lance_bench),
            true,
        ),
    ] {
        let time = ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let thrpt = ns
            .map(|n| fmt_throughput((N_DOCS as f64) / (n / 1e9)))
            .unwrap_or_else(|| "—".into());
        let peak = peak_rss.map(rss::fmt_bytes).unwrap_or_else(|| "—".into());
        let cmp = if is_baseline {
            "—".to_string()
        } else {
            fmt_winner("infino", ns, "lance", lance_ns)
        };
        body.push_str(&format!(
            "| {label} | {time} | {thrpt} | {peak} | {median_rss} | {p90_rss} | {rss_delta} | {cmp} |\n"
        ));
    }

    markdown::emit(&MarkdownSection {
        anchor_id: "bench/vector/supertable/ingest".into(),
        body,
    });
}

fn emit_search_markdown() {
    use markdown::{MarkdownSection, fmt_time, fmt_winner, read_infino_mean_ns, read_mean_ns};

    let cal = calibrations();
    let group = group_name::SUPERTABLE_VEC_SEARCH;

    let mut body = String::new();
    body.push_str(&format!(
        "### Supertable vector — search ({N_DOCS} docs × dim={DIM}, calibrated at recall targets)\n\n"
    ));
    body.push_str(
        "Infino hot/warm/cold from `../infino-pr8` (`supertable_vec_{hot|warm|cold}_search*`). \
         Lance hot = local `TempDir`; Lance warm/cold = `s3://` on s3s-fs or real S3 \
         (`supertable_vec_{warm|cold}_search_lance_{s3s_fs|real_s3}`). Winner = infino hot vs Lance hot.\n\n",
    );
    body.push_str(
        "| Recall target | infino hot | infino warm | infino cold | Lance hot | Lance warm | Lance cold | Lance (p,r) | Lance Peak RSS | Winner |\n",
    );
    body.push_str(
        "|---------------|------------|-------------|-------------|-----------|------------|------------|-------------|----------------|--------|\n",
    );

    for (i, &target) in RECALL_TARGETS.iter().enumerate() {
        let recall_label = format!("recall_at_least_{:02}", (target * 100.0) as u32);
        let row_target = format!("{target:.2}");

        // Supertable (infino): PR6 flat id; calibrated (p/seg, refine)
        // is in-memory inside infino's process and not on disk.
        let st_bid = format!("supertable_{recall_label}");
        let st_hot = read_infino_mean_ns(group, &st_bid);
        let st_warm =
            markdown::read_infino_supertable_tier_mean_ns("supertable_vec", "warm", &st_bid);
        let st_cold =
            markdown::read_infino_supertable_tier_mean_ns("supertable_vec", "cold", &st_bid);

        let (lan_cell, lan_hot, lan_warm, lan_cold, lan_peak) = match cal.lance[i] {
            Some(c) => {
                let r_u32 = c.refine as u32;
                let bid = format!("lance_{recall_label}/p={},r={}", c.probe, r_u32);
                let hot = read_mean_ns(group, &bid);
                let warm = markdown::read_lance_tier_mean_ns("supertable_vec", "warm", &bid);
                let cold = markdown::read_lance_tier_mean_ns("supertable_vec", "cold", &bid);
                let peak = rss::read_peak_rss_bytes(group, &bid);
                (
                    format!("(p={}, r={})", c.probe, r_u32),
                    hot,
                    warm,
                    cold,
                    peak,
                )
            }
            None => ("—".into(), None, None, None, None),
        };

        let winner = fmt_winner("supertable", st_hot, "lance", lan_hot);
        body.push_str(&format!(
            "| {row_target} | {} | {} | {} | {} | {} | {} | {lan_cell} | {} | {winner} |\n",
            st_hot.map(fmt_time).unwrap_or_else(|| "—".into()),
            st_warm.map(fmt_time).unwrap_or_else(|| "—".into()),
            st_cold.map(fmt_time).unwrap_or_else(|| "—".into()),
            lan_hot.map(fmt_time).unwrap_or_else(|| "—".into()),
            lan_warm.map(fmt_time).unwrap_or_else(|| "—".into()),
            lan_cold.map(fmt_time).unwrap_or_else(|| "—".into()),
            lan_peak.map(rss::fmt_bytes).unwrap_or_else(|| "—".into()),
        ));
    }

    markdown::emit(&MarkdownSection {
        anchor_id: "bench/vector/supertable/search".into(),
        body,
    });
}

criterion_group!(benches, bench);
