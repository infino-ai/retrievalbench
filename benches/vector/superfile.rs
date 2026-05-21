//! Lance head-to-head vector bench for the superfile layer.
//!
//! Measures Lance only — infino's own numbers come from `infino/benches`
//! (read out of `../infino/target/criterion/...` at emit time). The
//! corpus, queries, and ground truth are shared with infino's bench via
//! `infino::test_helpers::bench_corpus` (re-exported via
//! `retrievalbench::corpus`).
//!
//! 1M × 384 Gaussian planted-cluster corpus, normalized for cosine. The
//! supertable shape (10M × 384, sharded into N superfiles) lives in
//! `benches/vector/supertable.rs`.
//!
//! ## Workflow
//!
//! ```text
//! (cd ../infino && cargo bench --bench vector -- superfile_vec)   # populate infino numbers
//! cargo bench --bench vector -- superfile_vec                     # measure Lance + emit
//! ```

use std::hint::black_box;
use std::sync::OnceLock;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group};
use lancedb::Table;
use retrievalbench::rss;
use tempfile::TempDir;
use tokio::runtime::Runtime;

use retrievalbench::corpus::{self, Calibrated, DIM};
use retrievalbench::lance;
use retrievalbench::markdown;

// ─── Constants ────────────────────────────────────────────────────────

const N_DOCS: usize = 1_000_000;

const TOP_K: usize = 10;
const N_CORRECTNESS_QUERIES: usize = 20;
const N_CALIBRATION_QUERIES: usize = 100;

const CORRECTNESS_RECALL_FLOOR: f32 = 0.80;
const CORRECTNESS_LANCE_PROBES: usize = 64;
const CORRECTNESS_LANCE_REFINE: u32 = 256;

const RECALL_TARGETS: &[f32] = &[0.90, 0.95, 0.99];

const PROBES: &[usize] = &[1, 5, 10, 25, 50, 100, 200, 400, 800];
const REFINES_U32: &[u32] = &[1, 4, 16, 64, 256, 1024];

// ─── Fixtures ────────────────────────────────────────────────────────

static VECTORS: OnceLock<Vec<f32>> = OnceLock::new();
static QUERIES_CORRECTNESS: OnceLock<Vec<Vec<f32>>> = OnceLock::new();
static QUERIES_CALIBRATION: OnceLock<Vec<Vec<f32>>> = OnceLock::new();
static GROUND_TRUTH_CORRECTNESS: OnceLock<Vec<Vec<u32>>> = OnceLock::new();
static GROUND_TRUTH_CALIBRATION: OnceLock<Vec<Vec<u32>>> = OnceLock::new();
static LANCE: OnceLock<LanceHandles> = OnceLock::new();
static CALIBRATIONS: OnceLock<Calibrations> = OnceLock::new();

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
    LANCE.get_or_init(|| build_lance_handles(vectors()))
}

// ─── Lance builder ────────────────────────────────────────────────────

struct LanceHandles {
    table: Table,
    _dir: TempDir,
    rt: Runtime,
}

fn build_lance_handles(vectors: &[f32]) -> LanceHandles {
    let n_cent = corpus::n_cent(N_DOCS) as u32;
    let rt = Runtime::new().expect("tokio runtime");
    let dir = TempDir::new().expect("TempDir");
    let (table, _) = lance::build_lance_table(
        &rt,
        dir.path(),
        vectors,
        N_DOCS,
        n_cent,
        lance::default_n_sub_vectors(),
    );
    LanceHandles {
        table,
        _dir: dir,
        rt,
    }
}

// ─── Correctness ──────────────────────────────────────────────────────

fn assert_lance_self_consistent(lh: &LanceHandles) -> f32 {
    let qs = queries_correctness();
    let gt = ground_truth_correctness();
    let mut total_recall = 0.0_f32;
    for (q, truth) in qs.iter().zip(gt.iter()) {
        let hits = lance::search_lance(
            &lh.rt,
            &lh.table,
            q,
            TOP_K,
            CORRECTNESS_LANCE_PROBES,
            CORRECTNESS_LANCE_REFINE,
        );
        assert!(!hits.is_empty(), "Lance kNN must return hits; got empty");
        total_recall += corpus::recall_at_k(&hits, truth);
    }
    let mean_recall = total_recall / (qs.len() as f32);
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
            "[superfile_vec_search] calibrating Lance at recall targets {RECALL_TARGETS:?}..."
        );
        let mut lan: [Option<Calibrated>; 3] = [None; 3];
        for (i, &target) in RECALL_TARGETS.iter().enumerate() {
            lan[i] = lance::calibrate_lance(
                &lh.rt,
                &lh.table,
                qs,
                gt,
                target,
                PROBES,
                REFINES_U32,
                21,
                TOP_K,
            );
            eprintln!("  recall ≥ {target:.2} | lance: {:?}", lan[i]);
        }
        Calibrations { lance: lan }
    })
}

// ─── Bench entry ──────────────────────────────────────────────────────

fn bench(c: &mut Criterion) {
    eprintln!("[superfile_vec] correctness: building Lance ({N_DOCS} docs)...");
    let lh = lance_handles();
    let lance_recall = assert_lance_self_consistent(lh);
    eprintln!(
        "[superfile_vec] correctness OK: Lance recall@{TOP_K} = {lance_recall:.3} (≥ {CORRECTNESS_RECALL_FLOOR:.2})"
    );

    artifact_report(N_DOCS, corpus::n_cent(N_DOCS), &lh.rt);

    // ---- Ingest sub-bench (group: superfile_vec_build) -------------
    {
        let v = vectors();
        let n_cent = corpus::n_cent(N_DOCS) as u32;
        let rt = Runtime::new().expect("tokio runtime");

        let mut g = c.benchmark_group("superfile_vec_build");
        g.sample_size(10);
        g.throughput(Throughput::Elements(N_DOCS as u64));
        let rss_sample = rss::PeakSampler::start_default();

        g.bench_function(format!("lance_build_{N_DOCS}docs"), |b| {
            b.iter_with_large_drop(|| {
                let dir = TempDir::new().expect("tempdir");
                let (table, _) = lance::build_lance_table(
                    &rt,
                    dir.path(),
                    black_box(v),
                    N_DOCS,
                    n_cent,
                    lance::default_n_sub_vectors(),
                );
                (table, dir)
            });
        });
        g.finish();
        let peak = rss_sample.stop();
        let _ = rss::write_peak_rss(
            group_name::SUPERFILE_VEC_BUILD,
            &format!("lance_build_{N_DOCS}docs"),
            peak,
        );

        emit_ingest_markdown();
    }

    // ---- Search sub-bench (group: superfile_vec_search) ------------
    {
        let cal = calibrations();
        let qs = queries_calibration();

        let mut g = c.benchmark_group("superfile_vec_search");
        g.sample_size(10);
        let rss_sample = rss::PeakSampler::start_default();

        for (i, &target) in RECALL_TARGETS.iter().enumerate() {
            let label = format!("recall_at_least_{:02}", (target * 100.0) as u32);

            if let Some(c_lan) = cal.lance[i] {
                let r_u32 = c_lan.refine as u32;
                g.bench_with_input(
                    BenchmarkId::new(
                        format!("lance_{label}"),
                        format!("p={},r={}", c_lan.probe, r_u32),
                    ),
                    &(c_lan.probe, r_u32),
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
        let peak = rss_sample.stop();
        for (i, &target) in RECALL_TARGETS.iter().enumerate() {
            let label = format!("recall_at_least_{:02}", (target * 100.0) as u32);
            if let Some(c_lan) = cal.lance[i] {
                let bid = format!("lance_{label}/p={},r={}", c_lan.probe, c_lan.refine as u32);
                let _ = rss::write_peak_rss(group_name::SUPERFILE_VEC_SEARCH, &bid, peak);
            }
        }

        emit_search_markdown();
    }
}

// ─── Markdown summary emitters ────────────────────────────────────────

mod group_name {
    pub const SUPERFILE_VEC_BUILD: &str = "superfile_vec_build";
    pub const SUPERFILE_VEC_SEARCH: &str = "superfile_vec_search";
}

fn emit_ingest_markdown() {
    use markdown::{
        MarkdownSection, fmt_throughput, fmt_time, fmt_winner, read_infino_mean_ns, read_mean_ns,
    };

    let group = group_name::SUPERFILE_VEC_BUILD;
    let infino_ns = read_infino_mean_ns(group, &format!("infino_build_{N_DOCS}docs"));
    let lance_ns = read_mean_ns(group, &format!("lance_build_{N_DOCS}docs"));
    let infino_rss = rss::read_infino_peak_rss_bytes(group, &format!("infino_build_{N_DOCS}docs"));
    let lance_rss = rss::read_peak_rss_bytes(group, &format!("lance_build_{N_DOCS}docs"));

    let mut body = String::new();
    body.push_str(&format!(
        "### Superfile vector — ingest ({N_DOCS} docs × dim={DIM}, Gaussian planted clusters, cosine)\n\n"
    ));
    body.push_str("| Engine | Time | Throughput | Peak RSS | vs LanceDB |\n");
    body.push_str("|--------|------|------------|----------|------------|\n");
    for (label, ns, peak_rss, baseline, is_baseline) in [
        ("infino", infino_ns, infino_rss, lance_ns, false),
        ("lance", lance_ns, lance_rss, lance_ns, true),
    ] {
        let time = ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let thrpt = ns
            .map(|n| fmt_throughput((N_DOCS as f64) / (n / 1e9)))
            .unwrap_or_else(|| "—".into());
        let rss = peak_rss.map(rss::fmt_bytes).unwrap_or_else(|| "—".into());
        let cmp = if is_baseline {
            "—".to_string()
        } else {
            fmt_winner("infino", ns, "lance", baseline)
        };
        body.push_str(&format!("| {label} | {time} | {thrpt} | {rss} | {cmp} |\n"));
    }

    markdown::emit(&MarkdownSection {
        anchor_id: "bench/vector/superfile/ingest".into(),
        body,
    });
}

fn emit_search_markdown() {
    use markdown::{
        MarkdownSection, fmt_time, fmt_winner, read_infino_calibrated, read_infino_mean_ns,
        read_mean_ns,
    };

    let cal = calibrations();
    let group = group_name::SUPERFILE_VEC_SEARCH;

    let mut body = String::new();
    body.push_str(&format!(
        "### Superfile vector — search ({N_DOCS} docs × dim={DIM}, calibrated at recall targets)\n\n"
    ));
    body.push_str("| Recall target | infino (probe, refine) | infino p50 | infino RSS | Lance (probe, refine) | Lance p50 | Lance RSS | Winner |\n");
    body.push_str("|---------------|------------------------|------------|------------|-----------------------|-----------|-----------|--------|\n");

    for (i, &target) in RECALL_TARGETS.iter().enumerate() {
        let recall_label = format!("recall_at_least_{:02}", (target * 100.0) as u32);
        let row_target = format!("{target:.2}");

        // Infino: enumerate ../infino/target/criterion to discover the
        // (probe, refine) infino's bench picked for this recall floor.
        let (inf_cell, inf_ns, inf_rss) =
            match read_infino_calibrated(group, &format!("infino_{recall_label}")) {
                Some((p, r, ns)) => {
                    let bid = format!("infino_{recall_label}/p={p},r={r}");
                    let peak_rss = rss::read_infino_peak_rss_bytes(group, &bid);
                    (format!("(p={p}, r={r})"), Some(ns), peak_rss)
                }
                None => ("—".into(), None, None),
            };
        // Lance: from local criterion output + own calibration.
        let (lan_cell, lan_ns, lan_rss) = match cal.lance[i] {
            Some(c) => {
                let r_u32 = c.refine as u32;
                let bid = format!("lance_{recall_label}/p={},r={}", c.probe, r_u32);
                let ns = read_mean_ns(group, &bid);
                let peak_rss = rss::read_peak_rss_bytes(group, &bid);
                (format!("(p={}, r={})", c.probe, r_u32), ns, peak_rss)
            }
            None => ("—".into(), None, None),
        };

        let inf_t = inf_ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let lan_t = lan_ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let inf_rss = inf_rss.map(rss::fmt_bytes).unwrap_or_else(|| "—".into());
        let lan_rss = lan_rss.map(rss::fmt_bytes).unwrap_or_else(|| "—".into());
        let winner = fmt_winner("infino", inf_ns, "lance", lan_ns);
        body.push_str(&format!(
            "| {row_target} | {inf_cell} | {inf_t} | {inf_rss} | {lan_cell} | {lan_t} | {lan_rss} | {winner} |\n"
        ));
    }

    body.push('\n');
    body.push_str(
        "**infino default options** (`nprobe=8, rerank_mult=20` — user-facing latency baseline):\n\n",
    );
    body.push_str("| Metric | Value |\n");
    body.push_str("|--------|-------|\n");
    let def = read_infino_mean_ns(group, "infino_default_options_top10");
    let def_s = def.map(fmt_time).unwrap_or_else(|| "—".into());
    let def_rss = rss::read_infino_peak_rss_bytes(group, "infino_default_options_top10")
        .map(rss::fmt_bytes)
        .unwrap_or_else(|| "—".into());
    body.push_str(&format!("| infino_default_options_top10 | {def_s} |\n"));
    body.push_str(&format!(
        "| infino_default_options_top10_peak_rss | {def_rss} |\n"
    ));

    markdown::emit(&MarkdownSection {
        anchor_id: "bench/vector/superfile/search".into(),
        body,
    });
}

// ─── Artifact size + cold-load report (Lance only) ───────────────────

fn artifact_report(n: usize, n_cent: usize, rt: &Runtime) {
    use std::time::Instant;
    let v = vectors();

    let t0 = Instant::now();
    let dir = TempDir::new().expect("tempdir");
    let (table, _) = lance::build_lance_table(
        rt,
        dir.path(),
        v,
        n,
        n_cent as u32,
        lance::default_n_sub_vectors(),
    );
    let build_elapsed = t0.elapsed();

    // Lance writes to disk; size is the tempdir tree.
    let size = walkdir_size(dir.path());
    let size_mib = size as f64 / (1024.0 * 1024.0);

    let q = &queries_calibration()[0];
    let t0 = Instant::now();
    let _ = lance::search_lance(rt, &table, q, TOP_K, CORRECTNESS_LANCE_PROBES, 16);
    let first_q_elapsed = t0.elapsed();

    eprintln!(
        "\n--- artifact-size + cold-load report ({n} docs, {n_cent} clusters, dim={DIM}) ---"
    );
    eprintln!(
        "lance:   build {:>7.2}s  size {size_mib:>6.2} MiB  first-query {:>5.2} ms",
        build_elapsed.as_secs_f64(),
        first_q_elapsed.as_secs_f64() * 1e3,
    );

    // `table` and `dir` drop at end of scope — bench measurement
    // builds its own fresh handles per closure call. This pre-build
    // is purely for the cold-load report.
}

fn walkdir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let m = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if m.is_file() {
            total += m.len();
        } else if m.is_dir() {
            total += walkdir_size(&entry.path());
        }
    }
    total
}

criterion_group!(benches, bench);
