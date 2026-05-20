//! Lance head-to-head vector bench for the supertable layer.
//!
//! Measures Lance only — infino's own numbers come from `infino/benches`
//! (read out of `../infino/target/criterion/...` at emit time). The
//! corpus is shared with infino's bench via
//! `infino::test_helpers::bench_corpus`.
//!
//! 10M × 384-dim Gaussian planted-cluster corpus, normalized for cosine.
//! Lance's `num_partitions = N_CENT_TOTAL` is set to match the
//! aggregate cluster count that infino's supertable shards (the
//! supertable's per-segment IVF centroids sum to the same total).
//!
//! ## Workflow
//!
//! ```text
//! (cd ../infino && cargo bench --bench vector -- supertable_vec)   # populate infino numbers
//! cargo bench --bench vector -- supertable_vec                     # measure Lance + emit
//! ```

use std::hint::black_box;
use std::sync::OnceLock;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group};
use lancedb::Table;
use tempfile::TempDir;
use tokio::runtime::Runtime;

use crate::corpus::{Calibrated, DIM};

// ─── Constants ────────────────────────────────────────────────────────

const N_DOCS: usize = 10_000_000;
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

fn vectors() -> &'static [f32] {
    VECTORS.get_or_init(|| {
        crate::corpus::generate_vector_corpus(N_DOCS, crate::corpus::n_cent(N_DOCS), 1, true)
    })
}

fn queries_correctness() -> &'static [Vec<f32>] {
    QUERIES_CORRECTNESS.get_or_init(|| {
        crate::corpus::generate_realistic_queries(
            vectors(),
            N_DOCS,
            N_CORRECTNESS_QUERIES,
            17,
            true,
            0.05,
        )
    })
}

fn queries_calibration() -> &'static [Vec<f32>] {
    QUERIES_CALIBRATION.get_or_init(|| {
        crate::corpus::generate_realistic_queries(
            vectors(),
            N_DOCS,
            N_CALIBRATION_QUERIES,
            99,
            true,
            0.05,
        )
    })
}

fn ground_truth_correctness() -> &'static [Vec<u32>] {
    GROUND_TRUTH_CORRECTNESS.get_or_init(|| {
        crate::corpus::ground_truth(vectors(), N_DOCS, queries_correctness(), TOP_K)
    })
}

fn ground_truth_calibration() -> &'static [Vec<u32>] {
    GROUND_TRUTH_CALIBRATION.get_or_init(|| {
        crate::corpus::ground_truth(vectors(), N_DOCS, queries_calibration(), TOP_K)
    })
}

fn lance_handles() -> &'static LanceHandles {
    LANCE.get_or_init(build_lance_handles)
}

// ─── Lance builder ────────────────────────────────────────────────────

struct LanceHandles {
    table: Table,
    _dir: TempDir,
    rt: Runtime,
}

fn build_lance_handles() -> LanceHandles {
    let n_cent = crate::corpus::n_cent(N_DOCS) as u32;
    let rt = Runtime::new().expect("tokio runtime");
    let dir = TempDir::new().expect("tempdir");
    let (table, _) = crate::lance::build_lance_table(
        &rt,
        dir.path(),
        vectors(),
        N_DOCS,
        n_cent,
        crate::lance::default_n_sub_vectors(),
    );
    LanceHandles { table, _dir: dir, rt }
}

// ─── Correctness ──────────────────────────────────────────────────────

fn assert_lance_self_consistent(lh: &LanceHandles) -> f32 {
    let mean_recall = crate::lance::mean_recall_lance(
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
            l[i] = crate::lance::calibrate_lance(
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
    eprintln!("[supertable_vec] correctness: building Lance ({N_DOCS} docs)...");
    let lh = lance_handles();
    let recall = assert_lance_self_consistent(lh);
    eprintln!(
        "[supertable_vec] correctness OK: Lance recall@{TOP_K} = {recall:.3} (≥ {CORRECTNESS_RECALL_FLOOR:.2})"
    );

    // ---- Ingest sub-bench (group: supertable_vec_build) ------------
    {
        let v = vectors();
        let n_cent = crate::corpus::n_cent(N_DOCS) as u32;

        let mut g = c.benchmark_group("supertable_vec_build");
        g.sample_size(10);
        g.throughput(Throughput::Elements(N_DOCS as u64));

        let rt = Runtime::new().expect("tokio runtime");
        g.bench_function(format!("lance_{N_DOCS}docs"), |b| {
            b.iter_with_large_drop(|| {
                let dir = TempDir::new().expect("tempdir");
                let (table, _) = crate::lance::build_lance_table(
                    &rt,
                    dir.path(),
                    black_box(v),
                    N_DOCS,
                    n_cent,
                    crate::lance::default_n_sub_vectors(),
                );
                (table, dir)
            });
        });

        g.finish();

        emit_ingest_markdown();
    }

    // ---- Search sub-bench (group: supertable_vec_search) -----------
    {
        let cal = calibrations();
        let qs = queries_calibration();

        let mut g = c.benchmark_group("supertable_vec_search");
        g.sample_size(10);

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
                            let hits =
                                crate::lance::search_lance(&lh.rt, &lh.table, q, TOP_K, p, r);
                            black_box(hits)
                        });
                    },
                );
            }
        }

        g.finish();

        emit_search_markdown();
    }
}

// ─── Markdown summary emitters ────────────────────────────────────────

fn emit_ingest_markdown() {
    use crate::markdown::{
        MarkdownSection, fmt_throughput, fmt_time, fmt_winner, read_infino_mean_ns, read_mean_ns,
    };

    let group = "supertable_vec_build";
    let infino_bench = format!(
        "supertable_{N_DOCS}docs_{n_seg}superfiles",
        n_seg = 4
    );
    let infino_ns = read_infino_mean_ns(group, &infino_bench);
    let lance_ns = read_mean_ns(group, &format!("lance_{N_DOCS}docs"));

    let mut body = String::new();
    body.push_str(&format!(
        "### Supertable vector — ingest ({N_DOCS} docs × dim={DIM}, sharded into 4 superfiles)\n\n"
    ));
    body.push_str("| Engine | Time | Throughput | vs LanceDB |\n");
    body.push_str("|--------|------|------------|------------|\n");
    for (label, ns, baseline, is_baseline) in [
        ("supertable", infino_ns, lance_ns, false),
        ("lance", lance_ns, lance_ns, true),
    ] {
        let time = ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let thrpt = ns
            .map(|n| fmt_throughput((N_DOCS as f64) / (n / 1e9)))
            .unwrap_or_else(|| "—".into());
        let cmp = if is_baseline {
            "—".to_string()
        } else {
            fmt_winner("infino", ns, "lance", baseline)
        };
        body.push_str(&format!("| {label} | {time} | {thrpt} | {cmp} |\n"));
    }

    crate::markdown::emit(&MarkdownSection {
        anchor_id: "bench/vector/supertable/ingest".into(),
        body,
    });
}

fn emit_search_markdown() {
    use crate::markdown::{
        MarkdownSection, fmt_time, fmt_winner, read_infino_calibrated, read_mean_ns,
    };

    let cal = calibrations();
    let group = "supertable_vec_search";

    let mut body = String::new();
    body.push_str(&format!(
        "### Supertable vector — search ({N_DOCS} docs × dim={DIM}, calibrated at recall targets)\n\n"
    ));
    body.push_str("| Recall target | supertable (probe/seg, refine) | supertable p50 | Lance (probe, refine) | Lance p50 | Winner |\n");
    body.push_str("|---------------|--------------------------------|----------------|-----------------------|-----------|--------|\n");

    for (i, &target) in RECALL_TARGETS.iter().enumerate() {
        let recall_label = format!("recall_at_least_{:02}", (target * 100.0) as u32);
        let row_target = format!("{target:.2}");

        let (st_cell, st_ns) =
            match read_infino_calibrated(group, &format!("supertable_{recall_label}")) {
                Some((p, r, ns)) => (format!("(p={p}, r={r})"), Some(ns)),
                None => ("—".into(), None),
            };
        let (lan_cell, lan_ns) = match cal.lance[i] {
            Some(c) => {
                let r_u32 = c.refine as u32;
                let bid = format!("lance_{recall_label}/p={},r={}", c.probe, r_u32);
                let ns = read_mean_ns(group, &bid);
                (format!("(p={}, r={})", c.probe, r_u32), ns)
            }
            None => ("—".into(), None),
        };
        let st_t = st_ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let lan_t = lan_ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let winner = fmt_winner("supertable", st_ns, "lance", lan_ns);
        body.push_str(&format!(
            "| {row_target} | {st_cell} | {st_t} | {lan_cell} | {lan_t} | {winner} |\n"
        ));
    }

    crate::markdown::emit(&MarkdownSection {
        anchor_id: "bench/vector/supertable/search".into(),
        body,
    });
}

criterion_group!(benches, bench);
