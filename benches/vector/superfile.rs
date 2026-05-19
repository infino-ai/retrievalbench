//! Single-binary vector bench for the superfile layer:
//!
//!   ingest head-to-head infino vs LanceDB
//! + calibrated kNN search head-to-head at recall targets
//!   {0.90, 0.95, 0.99}
//! + infino-only `(nprobe, rerank_mult)` sweep for the
//!   recall-vs-latency curve
//! + correctness gate (both engines clear recall@10 ≥ 0.80 at the
//!   high-recall correctness config)
//! + one-shot artifact-size + cold-load + first-query report
//!
//! Pinned to 1M-doc × dim-384 Gaussian planted-cluster corpus,
//! normalized for cosine. Single-superfile shape is rarely much
//! larger in production — supertable scale-out at 10M+ docs lives
//! in `benches/vector/supertable.rs`.
//!
//! ## Apples-to-apples vs Lance
//!
//! - **Index family**: both IVF-based. infino = IVF + 1-bit RaBitQ
//!   + f32 rerank; Lance = IVF + 8-bit PQ + f32 refine.
//!   `nlist = num_partitions`. Lance's `num_sub_vectors` is
//!   `dim / 6 = 64`; each engine's quantization codes land at
//!   48–64 B / vector (same magnitude footprint).
//! - **Corpus**: identical bytes fed to both engines (normalized
//!   for cosine).
//! - **Calibration grid**: both engines pick from the same
//!   symmetric `(probe, refine)` grid — neither gets a coverage
//!   advantage.
//! - **Lance per-query overhead** that infino doesn't pay (every
//!   Lance call: `Vec<f32>` clone + `block_on` into Tokio +
//!   DataFusion plan build + record-batch downcast): ~70–260 µs
//!   per query, or 0.6–2.4% of an 11 ms Lance latency. Included
//!   in numbers — it's what a real Lance user pays via the public
//!   API. Not subtracted.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench vector -- superfile_vec_build      # ingest only
//! cargo bench --bench vector -- superfile_vec_search     # search only
//! INFINO_BENCH_UPDATE_README=1 cargo bench --bench vector   # rewrites results in benches/README.md
//! ```

use bytes::Bytes;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group};
use infino::superfile::vector::distance::Metric;
use infino::superfile::vector::reader::VectorReader;
use lancedb::Table;
use std::hint::black_box;
use std::sync::OnceLock;
use std::time::Instant;
use tempfile::TempDir;
use tokio::runtime::Runtime;

// ─── Constants ────────────────────────────────────────────────────────

/// Doc count for every vector-superfile bench. Pinned to 1M — a
/// single superfile in production is rarely much larger; the
/// multi-segment supertable shape (which scales linearly via
/// superfiles) is what `INFINO_BENCH_FULL=1` and the scale bundle
/// stress at 10M+ documents.
const N_DOCS: usize = 1_000_000;

const TOP_K: usize = 10;
const N_CORRECTNESS_QUERIES: usize = 20;
const N_CALIBRATION_QUERIES: usize = 100;

/// Lower bound on mean recall@10 at the **correctness config** (high
/// nprobe + high rerank_mult — see below). Catches catastrophic
/// regressions in the IVF + RaBitQ + rerank pipeline. Sub-0.80
/// recall at a generous config suggests a real clustering /
/// quantization / rerank bug, not a tuning issue.
const CORRECTNESS_RECALL_FLOOR: f32 = 0.80;

/// Correctness config: probes 64/1024 = 6.25% of the corpus,
/// reranks 2560 candidates. Well into the regime where any working
/// IVF + RaBitQ + rerank pipeline returns recall ≥ 0.95 on
/// planted-cluster data.
const CORRECTNESS_NPROBE: usize = 64;
const CORRECTNESS_RERANK_MULT: usize = 256;

/// Lance counterpart for the correctness check.
const CORRECTNESS_LANCE_PROBES: usize = 64;
const CORRECTNESS_LANCE_REFINE: u32 = 256;

/// Default options — the user-facing latency baseline. The
/// `(nprobe, rerank_mult)` sweep below reports the
/// recall-vs-latency curve around this point.
const DEFAULT_NPROBE: usize = 8;
const DEFAULT_RERANK_MULT: usize = 20;

const RECALL_TARGETS: &[f32] = &[0.90, 0.95, 0.99];

// Symmetric (probe, refine) calibration grids — each engine is
// allowed to pick from the same set of knob positions, so neither
// has a grid-coverage advantage.
const PROBES: &[usize] = &[1, 5, 10, 25, 50, 100, 200, 400, 800];
const REFINES_USIZE: &[usize] = &[1, 4, 16, 64, 256, 1024];
const REFINES_U32: &[u32] = &[1, 4, 16, 64, 256, 1024];

// ─── Fixtures (built once, reused across criterion samples) ────────────

static VECTORS: OnceLock<Vec<f32>> = OnceLock::new();
static QUERIES_CORRECTNESS: OnceLock<Vec<Vec<f32>>> = OnceLock::new();
static QUERIES_CALIBRATION: OnceLock<Vec<Vec<f32>>> = OnceLock::new();
static GROUND_TRUTH_CORRECTNESS: OnceLock<Vec<Vec<u32>>> = OnceLock::new();
static GROUND_TRUTH_CALIBRATION: OnceLock<Vec<Vec<u32>>> = OnceLock::new();
static INFINO_BLOB: OnceLock<Vec<u8>> = OnceLock::new();
static LANCE: OnceLock<LanceHandles> = OnceLock::new();
static CALIBRATIONS: OnceLock<Calibrations> = OnceLock::new();

fn vectors() -> &'static [f32] {
    VECTORS.get_or_init(|| {
        crate::corpus::generate_vector_corpus(N_DOCS, crate::corpus::n_cent(N_DOCS), 1, true)
    })
}

fn queries_correctness() -> &'static [Vec<f32>] {
    QUERIES_CORRECTNESS.get_or_init(|| {
        crate::lance::generate_realistic_queries(vectors(), N_DOCS, N_CORRECTNESS_QUERIES, 17, true, 0.05)
    })
}

fn queries_calibration() -> &'static [Vec<f32>] {
    QUERIES_CALIBRATION.get_or_init(|| {
        crate::lance::generate_realistic_queries(vectors(), N_DOCS, N_CALIBRATION_QUERIES, 99, true, 0.05)
    })
}

fn ground_truth_correctness() -> &'static [Vec<u32>] {
    GROUND_TRUTH_CORRECTNESS.get_or_init(|| {
        crate::lance::ground_truth(vectors(), N_DOCS, queries_correctness(), TOP_K)
    })
}

fn ground_truth_calibration() -> &'static [Vec<u32>] {
    GROUND_TRUTH_CALIBRATION.get_or_init(|| {
        crate::lance::ground_truth(vectors(), N_DOCS, queries_calibration(), TOP_K)
    })
}

fn infino_reader() -> VectorReader {
    let blob = INFINO_BLOB.get_or_init(|| build_infino_blob(vectors()));
    open_infino_reader(blob.clone())
}

fn lance_handles() -> &'static LanceHandles {
    LANCE.get_or_init(|| build_lance_handles(vectors()))
}

// ─── Builders — infino ────────────────────────────────────────────────

fn build_infino_blob(vectors: &[f32]) -> Vec<u8> {
    let n_cent = crate::corpus::n_cent(N_DOCS);
    let builder = crate::corpus::build_vector_index(vectors, N_DOCS, n_cent, Metric::Cosine);
    builder.finish()
}

fn open_infino_reader(blob: Vec<u8>) -> VectorReader {
    let n_cent = crate::corpus::n_cent(N_DOCS);
    let json = format!(
        r#"[{{"name":"v","dim":{},"n_cent":{n_cent},"rot_seed":7,"metric":"cosine"}}]"#,
        crate::lance::DIM
    );
    VectorReader::open(Bytes::from(blob), &json).expect("open VectorReader")
}

// ─── Builders — Lance ─────────────────────────────────────────────────

struct LanceHandles {
    table: Table,
    _dir: TempDir,
    rt: Runtime,
}

fn build_lance_handles(vectors: &[f32]) -> LanceHandles {
    let n_cent = crate::corpus::n_cent(N_DOCS) as u32;
    let rt = Runtime::new().expect("tokio runtime");
    let dir = TempDir::new().expect("TempDir");
    let (table, _) = crate::lance::build_lance_table(
        &rt,
        dir.path(),
        vectors,
        N_DOCS,
        n_cent,
        crate::lance::default_n_sub_vectors(),
    );
    LanceHandles { table, _dir: dir, rt }
}

// ─── Correctness ──────────────────────────────────────────────────────

fn assert_infino_self_consistent(reader: &VectorReader) -> f32 {
    let qs = queries_correctness();
    let gt = ground_truth_correctness();
    let mut total_recall = 0.0_f32;
    for (q, truth) in qs.iter().zip(gt.iter()) {
        let hits = reader
            .search("v", q, TOP_K, CORRECTNESS_NPROBE, CORRECTNESS_RERANK_MULT)
            .expect("vector search");
        assert_eq!(
            hits.len(),
            TOP_K,
            "infino kNN should fill top-{TOP_K}; got {}",
            hits.len()
        );
        total_recall += crate::lance::recall_at_k(&hits, truth);
    }
    let mean_recall = total_recall / (qs.len() as f32);
    assert!(
        mean_recall >= CORRECTNESS_RECALL_FLOOR,
        "infino mean recall@{TOP_K} at correctness config \
         (p={CORRECTNESS_NPROBE}, r={CORRECTNESS_RERANK_MULT}) \
         below floor: {mean_recall:.3} < {CORRECTNESS_RECALL_FLOOR:.3}"
    );
    mean_recall
}

fn assert_lance_self_consistent(lh: &LanceHandles) -> f32 {
    let qs = queries_correctness();
    let gt = ground_truth_correctness();
    let mut total_recall = 0.0_f32;
    for (q, truth) in qs.iter().zip(gt.iter()) {
        let hits = crate::lance::search_lance(
            &lh.rt,
            &lh.table,
            q,
            TOP_K,
            CORRECTNESS_LANCE_PROBES,
            CORRECTNESS_LANCE_REFINE,
        );
        assert!(!hits.is_empty(), "Lance kNN must return hits; got empty");
        total_recall += crate::lance::recall_at_k(&hits, truth);
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
    infino: [Option<crate::lance::Calibrated>; 3],
    lance: [Option<crate::lance::Calibrated>; 3],
}

fn calibrations() -> &'static Calibrations {
    CALIBRATIONS.get_or_init(|| {
        let reader = infino_reader();
        let lh = lance_handles();
        let qs = queries_calibration();
        let gt = ground_truth_calibration();

        eprintln!(
            "[superfile_vec_search] calibrating infino + Lance at recall targets {:?}...",
            RECALL_TARGETS
        );
        let mut inf = [None; 3];
        let mut lan = [None; 3];
        for (i, &target) in RECALL_TARGETS.iter().enumerate() {
            inf[i] = crate::lance::calibrate_infino(
                &reader,
                qs,
                gt,
                target,
                PROBES,
                REFINES_USIZE,
                21,
                TOP_K,
            );
            lan[i] = crate::lance::calibrate_lance(
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
            eprintln!("  recall ≥ {target:.2} | infino: {:?} | lance: {:?}", inf[i], lan[i]);
        }
        Calibrations { infino: inf, lance: lan }
    })
}

// ─── Bench entry ──────────────────────────────────────────────────────

fn bench(c: &mut Criterion) {
    // ---- Correctness phase (runs regardless of criterion filter) ---
    eprintln!("[superfile_vec] correctness: building infino + Lance ({N_DOCS} docs)...");
    let reader = infino_reader();
    let lh = lance_handles();
    let infino_recall = assert_infino_self_consistent(&reader);
    let lance_recall = assert_lance_self_consistent(lh);
    eprintln!(
        "[superfile_vec] correctness OK: infino recall@{TOP_K} = {infino_recall:.3}, \
         Lance recall@{TOP_K} = {lance_recall:.3} (both ≥ {CORRECTNESS_RECALL_FLOOR:.2})"
    );

    // ---- Ingest sub-bench (group: superfile_vec_build) -------------
    {
        let n_cent = crate::corpus::n_cent(N_DOCS);
        let v = vectors();

        let mut g = c.benchmark_group("superfile_vec_build");
        g.sample_size(10);
        g.throughput(Throughput::Elements(N_DOCS as u64));

        g.bench_function(format!("infino_build_{N_DOCS}docs"), |b| {
            b.iter_with_large_drop(|| build_infino_blob(black_box(v)));
        });

        let rt = Runtime::new().expect("tokio runtime");
        g.bench_function(format!("lance_build_{N_DOCS}docs"), |b| {
            b.iter_with_large_drop(|| {
                let dir = TempDir::new().expect("TempDir");
                let (table, _) = crate::lance::build_lance_table(
                    &rt,
                    dir.path(),
                    black_box(v),
                    N_DOCS,
                    n_cent as u32,
                    crate::lance::default_n_sub_vectors(),
                );
                (table, dir)
            });
        });

        g.finish();

        artifact_report(N_DOCS, n_cent, v, &rt);
        emit_ingest_markdown();
    }

    // ---- Search sub-bench (group: superfile_vec_search) -------------
    {
        let cal = calibrations();
        let qs = queries_calibration();

        let mut g = c.benchmark_group("superfile_vec_search");
        g.sample_size(10);

        for (i, &target) in RECALL_TARGETS.iter().enumerate() {
            let label = format!("recall_at_least_{:02}", (target * 100.0) as u32);

            if let Some(c_inf) = cal.infino[i] {
                g.bench_with_input(
                    BenchmarkId::new(
                        format!("infino_{label}"),
                        format!("p={},r={}", c_inf.probe, c_inf.refine),
                    ),
                    &(c_inf.probe, c_inf.refine),
                    |b, &(p, r)| {
                        let q = &qs[0];
                        b.iter(|| {
                            let hits = reader
                                .search("v", black_box(q), TOP_K, p, r)
                                .expect("kNN search");
                            black_box(hits)
                        });
                    },
                );
            }
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
                            let hits = crate::lance::search_lance(&lh.rt, &lh.table, q, TOP_K, p, r);
                            black_box(hits)
                        });
                    },
                );
            }
        }

        // ---- Infino-only (nprobe, rerank_mult) sweep ----------------
        let q = &qs[0];
        let n_cent = crate::corpus::n_cent(N_DOCS);

        g.bench_function("infino_default_options_top10", |b| {
            b.iter(|| {
                let hits = reader
                    .search(
                        black_box("v"),
                        black_box(q),
                        TOP_K,
                        DEFAULT_NPROBE,
                        DEFAULT_RERANK_MULT,
                    )
                    .expect("kNN search");
                black_box(hits)
            });
        });

        for &nprobe in &[1, 4, 8, 16, 32, 64, 128] {
            if nprobe > n_cent {
                continue;
            }
            g.bench_with_input(
                BenchmarkId::new("infino_nprobe_sweep_rerank20", nprobe),
                &nprobe,
                |b, &np| {
                    b.iter(|| {
                        let hits = reader
                            .search(black_box("v"), black_box(q), TOP_K, np, DEFAULT_RERANK_MULT)
                            .expect("kNN search");
                        black_box(hits)
                    });
                },
            );
        }

        for &rerank in &[1, 5, 10, 20, 50, 100] {
            g.bench_with_input(
                BenchmarkId::new("infino_rerank_sweep_nprobe8", rerank),
                &rerank,
                |b, &rm| {
                    b.iter(|| {
                        let hits = reader
                            .search(black_box("v"), black_box(q), TOP_K, DEFAULT_NPROBE, rm)
                            .expect("kNN search");
                        black_box(hits)
                    });
                },
            );
        }

        g.finish();

        emit_search_markdown();
    }
}

// ─── Markdown summary emitters ────────────────────────────────────────

fn emit_ingest_markdown() {
    use crate::markdown::{MarkdownSection, fmt_throughput, fmt_time, fmt_winner, read_mean_ns};

    let group = "superfile_vec_build";
    let infino_ns = read_mean_ns(group, &format!("infino_build_{N_DOCS}docs"));
    let lance_ns = read_mean_ns(group, &format!("lance_build_{N_DOCS}docs"));

    let mut body = String::new();
    body.push_str(&format!(
        "### Superfile vector — ingest ({N_DOCS} docs × dim={}, Gaussian planted clusters, cosine)\n\n",
        crate::lance::DIM
    ));
    body.push_str("| Engine | Time | Throughput | vs LanceDB |\n");
    body.push_str("|--------|------|------------|------------|\n");
    for (label, ns, baseline, is_baseline) in [
        ("infino", infino_ns, lance_ns, false),
        ("lance",  lance_ns,  lance_ns, true),
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
        anchor_id: "bench/vector/superfile/ingest".into(),
        body,
    });
}

fn emit_search_markdown() {
    use crate::markdown::{MarkdownSection, fmt_time, fmt_winner, read_mean_ns};

    let cal = calibrations();
    let group = "superfile_vec_search";

    let mut body = String::new();
    body.push_str(&format!(
        "### Superfile vector — search ({N_DOCS} docs × dim={}, calibrated at recall targets)\n\n",
        crate::lance::DIM
    ));
    body.push_str("| Recall target | infino (probe, refine) | infino p50 | Lance (probe, refine) | Lance p50 | Winner |\n");
    body.push_str("|---------------|------------------------|------------|-----------------------|-----------|--------|\n");

    for (i, &target) in RECALL_TARGETS.iter().enumerate() {
        let label = format!("recall_at_least_{:02}", (target * 100.0) as u32);
        let (inf_cell, inf_ns) = match cal.infino[i] {
            Some(c) => {
                let bid = format!("infino_{label}/p={},r={}", c.probe, c.refine);
                let ns = read_mean_ns(group, &bid);
                let cell = format!("(p={}, r={})", c.probe, c.refine);
                (cell, ns)
            }
            None => ("—".into(), None),
        };
        let (lan_cell, lan_ns) = match cal.lance[i] {
            Some(c) => {
                let r_u32 = c.refine as u32;
                let bid = format!("lance_{label}/p={},r={}", c.probe, r_u32);
                let ns = read_mean_ns(group, &bid);
                let cell = format!("(p={}, r={})", c.probe, r_u32);
                (cell, ns)
            }
            None => ("—".into(), None),
        };
        let inf_t = inf_ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let lan_t = lan_ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let winner = fmt_winner("infino", inf_ns, "lance", lan_ns);
        body.push_str(&format!(
            "| {:.2} | {} | {} | {} | {} | {} |\n",
            target, inf_cell, inf_t, lan_cell, lan_t, winner
        ));
    }

    body.push_str("\n**infino default options** (`nprobe=8, rerank_mult=20` — user-facing latency baseline):\n\n");
    body.push_str("| Metric | Value |\n");
    body.push_str("|--------|-------|\n");
    let default_ns = read_mean_ns(group, "infino_default_options_top10");
    let default_s = default_ns.map(fmt_time).unwrap_or_else(|| "—".into());
    body.push_str(&format!("| infino_default_options_top10 | {default_s} |\n"));

    crate::markdown::emit(&MarkdownSection {
        anchor_id: "bench/vector/superfile/search".into(),
        body,
    });
}

// ─── Artifact size + cold-load + first-query report ──────────────────

fn artifact_report(n: usize, n_cent: usize, v: &[f32], rt: &Runtime) {
    eprintln!(
        "\n--- artifact-size + cold-load report ({n} docs, {n_cent} clusters, dim={}) ---",
        crate::lance::DIM
    );

    let t0 = Instant::now();
    let blob = crate::corpus::build_vector_index(v, n, n_cent, Metric::Cosine).finish();
    let infino_build = t0.elapsed();
    let infino_bytes = blob.len();

    let t0 = Instant::now();
    let reader = {
        let json = format!(
            r#"[{{"name":"v","dim":{},"n_cent":{n_cent},"rot_seed":7,"metric":"cosine"}}]"#,
            crate::lance::DIM
        );
        VectorReader::open(Bytes::from(blob), &json).expect("open VectorReader")
    };
    let infino_open = t0.elapsed();

    let q = crate::lance::generate_realistic_queries(v, n, 1, 99, true, 0.05);
    let t0 = Instant::now();
    let _ = reader.search("v", &q[0], 10, 5, 1024).expect("kNN search");
    let infino_first_query = t0.elapsed();

    eprintln!(
        "infino:  build {:>7.2}s  size {:>5.2} MiB  open {:>5.2} ms  first-query {:>5.2} ms",
        infino_build.as_secs_f32(),
        infino_bytes as f32 / (1024.0 * 1024.0),
        infino_open.as_secs_f32() * 1e3,
        infino_first_query.as_secs_f32() * 1e3,
    );

    let dir = TempDir::new().expect("TempDir");
    let t0 = Instant::now();
    let (table, lance_build_dur) = crate::lance::build_lance_table(
        rt,
        dir.path(),
        v,
        n,
        n_cent as u32,
        crate::lance::default_n_sub_vectors(),
    );
    let lance_total = t0.elapsed();
    drop(table);
    let lance_size = walkdir_size(dir.path());

    let t0 = Instant::now();
    let table = rt.block_on(async {
        let db = lancedb::connect(dir.path().to_str().expect("path to_str"))
            .execute()
            .await
            .expect("connect");
        db.open_table("v").execute().await.expect("open_table")
    });
    let lance_open = t0.elapsed();
    let t0 = Instant::now();
    let _ = crate::lance::search_lance(rt, &table, &q[0], 10, 10, 256);
    let lance_first_query = t0.elapsed();

    eprintln!(
        "lance:   build {:>7.2}s (inner {:>5.2}s)  size {:>5.2} MiB  open {:>5.2} ms  first-query {:>5.2} ms",
        lance_total.as_secs_f32(),
        lance_build_dur.as_secs_f32(),
        lance_size as f32 / (1024.0 * 1024.0),
        lance_open.as_secs_f32() * 1e3,
        lance_first_query.as_secs_f32() * 1e3,
    );
}

fn walkdir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                total += walkdir_size(&p);
            } else if let Ok(m) = std::fs::metadata(&p) {
                total += m.len();
            }
        }
    }
    total
}

criterion_group!(benches, bench);
