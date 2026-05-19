//! Single-binary vector bench for the supertable layer:
//!
//!   ingest head-to-head infino-supertable vs LanceDB
//! + calibrated kNN search head-to-head at recall targets
//!   {0.90, 0.95, 0.99}
//! + correctness gate (both engines clear recall@10 ≥ 0.80 at the
//!   high-recall correctness config)
//!
//! 10M × 384-dim Gaussian planted-cluster corpus, normalized for
//! cosine. The supertable shards into [`N_SEGMENTS`] superfiles with
//! `n_cent_per_segment = N_CENT_TOTAL / N_SEGMENTS`, so the total
//! IVF cluster count matches Lance's `num_partitions = N_CENT_TOTAL`
//! — apples-to-apples cluster coverage.
//!
//! ## Apples-to-apples vs Lance
//!
//! - **Total cluster coverage**: supertable's per-segment IVF
//!   centroids sum to `N_CENT_TOTAL = Lance.num_partitions`. Same
//!   total clusters per corpus; supertable distributes them across
//!   superfiles.
//! - **Quantization footprint**: 1-bit RaBitQ on infino, 8-bit PQ
//!   (64 sub-vectors) on Lance — both land at the same 48–64 B /
//!   vector magnitude.
//! - **Calibration grid**: each engine picks from its own probe /
//!   refine grid; supertable's `nprobe` is per-segment so the
//!   effective probe count is `nprobe × n_superfiles` (the grid
//!   below is scaled down by [`N_SEGMENTS`] to reflect that).
//! - **Same brute-force ground truth**: 100-query calibration
//!   battery + 20-query correctness battery, identical for both
//!   engines.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench vector -- supertable_vec_build      # ingest only
//! cargo bench --bench vector -- supertable_vec_search     # search only
//! ```

use std::hint::black_box;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use arrow_array::{Array, FixedSizeListArray, Float32Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group};
use infino::superfile::builder::VectorConfig;
use infino::superfile::vector::distance::Metric;
use infino::supertable::query::SuperfileHit;
use infino::supertable::query::vector::VectorSearchOptions;
use infino::supertable::{Supertable, SupertableOptions};
use lancedb::Table;
use tempfile::TempDir;
use tokio::runtime::Runtime;

// ─── Constants ────────────────────────────────────────────────────────

/// Doc count for every vector-supertable bench. Pinned to 10M —
/// supertable is the scale-out shape.
const N_DOCS: usize = 10_000_000;

const N_SEGMENTS: usize = 4;
const TOP_K: usize = 10;

const N_CORRECTNESS_QUERIES: usize = 20;
const N_CALIBRATION_QUERIES: usize = 100;

const RECALL_TARGETS: &[f32] = &[0.90, 0.95, 0.99];

/// Per-segment probe grid: the supertable runs `vector_search` with
/// a single `nprobe` value applied to every segment, so the
/// effective probe count is `nprobe × n_superfiles`. Sweep
/// per-segment values that approximate the same effective coverage
/// as the single-superfile probe grid scaled down by [`N_SEGMENTS`].
const SUPERTABLE_PROBES_PER_SEG: &[usize] = &[1, 2, 4, 8, 12, 16];
const SUPERTABLE_REFINES: &[usize] = &[4, 16, 64, 256, 1024];

const LANCE_PROBES: &[usize] = &[1, 5, 10, 25, 50, 100, 200, 400, 800];
const LANCE_REFINES: &[u32] = &[1, 4, 16, 64, 256, 1024];

/// Correctness config — high-recall point for both engines.
const CORRECTNESS_RECALL_FLOOR: f32 = 0.80;
const CORRECTNESS_SUPERTABLE_NPROBE: usize = 16;
const CORRECTNESS_SUPERTABLE_RERANK_MULT: usize = 256;
const CORRECTNESS_LANCE_PROBES: usize = 64;
const CORRECTNESS_LANCE_REFINE: u32 = 256;

// ─── Fixtures (built once, reused across criterion samples) ────────────

static VECTORS: OnceLock<Vec<f32>> = OnceLock::new();
static QUERIES_CORRECTNESS: OnceLock<Vec<Vec<f32>>> = OnceLock::new();
static QUERIES_CALIBRATION: OnceLock<Vec<Vec<f32>>> = OnceLock::new();
static GROUND_TRUTH_CORRECTNESS: OnceLock<Vec<Vec<u32>>> = OnceLock::new();
static GROUND_TRUTH_CALIBRATION: OnceLock<Vec<Vec<u32>>> = OnceLock::new();
static SUPERTABLE: OnceLock<Supertable> = OnceLock::new();
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

fn supertable() -> &'static Supertable {
    SUPERTABLE.get_or_init(build_supertable)
}

fn lance_handles() -> &'static LanceHandles {
    LANCE.get_or_init(build_lance_handles)
}

// ─── Builders — infino supertable ─────────────────────────────────────

fn supertable_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![Field::new(
        "emb",
        DataType::FixedSizeList(
            Arc::new(Field::new("item", DataType::Float32, true)),
            crate::lance::DIM as i32,
        ),
        false,
    )]))
}

fn build_supertable() -> Supertable {
    let n_cent_total = crate::corpus::n_cent(N_DOCS);
    let n_cent_per_segment = (n_cent_total / N_SEGMENTS).max(1);

    // rayon-default pool — internal commit-time work fans out
    // across cores.
    let pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_cpus::get().max(1))
            .build()
            .expect("pool"),
    );
    let opts = SupertableOptions::new(
        supertable_schema(),
        vec![],
        vec![VectorConfig {
            column: "emb".into(),
            dim: crate::lance::DIM,
            n_cent: n_cent_per_segment,
            rot_seed: 7,
            metric: Metric::Cosine,
        }],
        None,
    )
    .expect("opts")
    .with_writer_pool(pool);

    let st = Supertable::create(opts);
    let mut w = st.writer().expect("writer");

    let chunk_size = N_DOCS / N_SEGMENTS;
    let v = vectors();
    for chunk_idx in 0..N_SEGMENTS {
        let start = chunk_idx * chunk_size;
        let end = start + chunk_size;
        let flat: Vec<f32> = v[start * crate::lance::DIM..end * crate::lance::DIM].to_vec();
        let item_field = Arc::new(Field::new("item", DataType::Float32, true));
        let values = Float32Array::from(flat);
        let fsl = FixedSizeListArray::try_new(
            item_field,
            crate::lance::DIM as i32,
            Arc::new(values) as Arc<dyn Array>,
            None,
        )
        .expect("FSL");
        let batch =
            RecordBatch::try_new(supertable_schema(), vec![Arc::new(fsl)]).expect("batch");
        w.append(&batch).expect("append");
        w.commit().expect("commit");
    }
    drop(w);
    st
}

/// Run a supertable kNN search and resolve per-superfile hits to
/// global doc-ids via cumulative-`n_docs` offsets in manifest order.
///
/// **Important**: `commit()` row-shards into
/// `min(writer_pool.threads, total_rows)` superfiles, so the bench
/// can't assume "one superfile per append batch." We compute a
/// prefix sum of `entry.n_docs` over the manifest's superfile list
/// (which preserves the row-range insertion order from
/// `publish_superfiles`); the global doc id for `(superfile, local)`
/// is `prefix_sum[superfile_idx] + local`.
fn supertable_topk(
    st: &Supertable,
    query: &[f32],
    k: usize,
    options: VectorSearchOptions,
) -> Vec<u32> {
    let r = st.reader();
    let hits: Vec<SuperfileHit> = r
        .vector_search("emb", query, k, options)
        .expect("vector_search");
    let manifest = r.manifest();
    // Cumulative offset per superfile in manifest insertion order.
    // `publish_superfiles` preserves the row-balanced shard order
    // emitted by `commit()`'s par_iter, so prefix-sum gives the
    // global doc-id base for each superfile.
    let mut offsets: Vec<u32> = Vec::with_capacity(manifest.superfiles.len());
    let mut acc: u32 = 0;
    for entry in manifest.superfiles.iter() {
        offsets.push(acc);
        acc = acc.saturating_add(entry.n_docs as u32);
    }
    hits.into_iter()
        .map(|h| {
            let seg_idx = manifest
                .superfiles
                .iter()
                .position(|e| e.uri == h.segment)
                .expect("superfile in manifest");
            offsets[seg_idx] + h.local_doc_id
        })
        .collect()
}

fn mean_recall_supertable(
    st: &Supertable,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
    options: VectorSearchOptions,
) -> f32 {
    let mut sum = 0f32;
    for (q, t) in queries.iter().zip(truths) {
        let hits = supertable_topk(st, q, TOP_K, options);
        let truth_set: std::collections::HashSet<u32> = t.iter().copied().collect();
        let recall = if t.is_empty() {
            1.0
        } else {
            hits.iter().filter(|id| truth_set.contains(id)).count() as f32 / t.len() as f32
        };
        sum += recall;
    }
    sum / queries.len() as f32
}

// ─── Builders — Lance ─────────────────────────────────────────────────

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

fn mean_recall_lance(
    lh: &LanceHandles,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
    probes: usize,
    refine: u32,
) -> f32 {
    crate::lance::mean_recall_lance(&lh.rt, &lh.table, queries, truths, TOP_K, probes, refine)
}

// ─── Correctness ──────────────────────────────────────────────────────

fn assert_supertable_self_consistent(st: &Supertable) -> f32 {
    let opts = VectorSearchOptions::new()
        .with_nprobe(CORRECTNESS_SUPERTABLE_NPROBE)
        .with_rerank_mult(CORRECTNESS_SUPERTABLE_RERANK_MULT);
    let mean_recall = mean_recall_supertable(
        st,
        queries_correctness(),
        ground_truth_correctness(),
        opts,
    );
    assert!(
        mean_recall >= CORRECTNESS_RECALL_FLOOR,
        "supertable mean recall@{TOP_K} at correctness config \
         (p={CORRECTNESS_SUPERTABLE_NPROBE}, r={CORRECTNESS_SUPERTABLE_RERANK_MULT}) \
         below floor: {mean_recall:.3} < {CORRECTNESS_RECALL_FLOOR:.3}"
    );
    mean_recall
}

fn assert_lance_self_consistent(lh: &LanceHandles) -> f32 {
    let mean_recall = mean_recall_lance(
        lh,
        queries_correctness(),
        ground_truth_correctness(),
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
    supertable: [Option<crate::lance::Calibrated>; 3],
    lance: [Option<crate::lance::Calibrated>; 3],
}

fn calibrate_supertable(
    st: &Supertable,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
    target_recall: f32,
) -> Option<crate::lance::Calibrated> {
    let mut best: Option<crate::lance::Calibrated> = None;
    let mut peak_recall = 0f32;
    for &probe in SUPERTABLE_PROBES_PER_SEG {
        for &refine in SUPERTABLE_REFINES {
            let opts = VectorSearchOptions::new()
                .with_nprobe(probe)
                .with_rerank_mult(refine);
            let recall = mean_recall_supertable(st, queries, truths, opts);
            if recall > peak_recall {
                peak_recall = recall;
            }
            if recall < target_recall {
                continue;
            }
            let q = &queries[0];
            let n_iter = 21;
            let mut samples = Vec::with_capacity(n_iter);
            for _ in 0..n_iter {
                let t0 = Instant::now();
                let _ = supertable_topk(st, q, TOP_K, opts);
                samples.push(t0.elapsed().as_secs_f32() * 1e6);
            }
            samples.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let p50 = samples[samples.len() / 2];
            let cand = crate::lance::Calibrated {
                probe,
                refine,
                recall,
                p50_micros: p50,
            };
            best = match best {
                None => Some(cand),
                Some(b) if cand.p50_micros < b.p50_micros => Some(cand),
                Some(b) => Some(b),
            };
        }
    }
    if best.is_none() {
        eprintln!(
            "    [supertable] no point hit recall ≥ {target_recall:.2}; peak observed = {peak_recall:.3}"
        );
    }
    best
}

fn calibrations() -> &'static Calibrations {
    CALIBRATIONS.get_or_init(|| {
        let st = supertable();
        let lh = lance_handles();
        let qs = queries_calibration();
        let gt = ground_truth_calibration();

        eprintln!(
            "[supertable_vec_search] calibrating supertable + Lance at recall targets {:?}...",
            RECALL_TARGETS
        );
        let mut s = [None; 3];
        let mut l = [None; 3];
        for (i, &target) in RECALL_TARGETS.iter().enumerate() {
            s[i] = calibrate_supertable(st, qs, gt, target);
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
            eprintln!("  recall ≥ {target:.2} | supertable: {:?} | lance: {:?}", s[i], l[i]);
        }
        Calibrations { supertable: s, lance: l }
    })
}

// ─── Bench entry ──────────────────────────────────────────────────────

fn bench(c: &mut Criterion) {
    // ---- Correctness phase ----------------------------------------
    eprintln!(
        "[supertable_vec] correctness: building supertable + Lance \
         ({N_DOCS} docs × {N_SEGMENTS} superfiles)..."
    );
    let st = supertable();
    let st_recall = assert_supertable_self_consistent(st);
    let lh = lance_handles();
    let lance_recall = assert_lance_self_consistent(lh);
    eprintln!(
        "[supertable_vec] correctness OK: supertable recall@{TOP_K} = {st_recall:.3}, \
         Lance recall@{TOP_K} = {lance_recall:.3} (both ≥ {CORRECTNESS_RECALL_FLOOR:.2})"
    );

    // ---- Ingest sub-bench (group: supertable_vec_build) ------------
    {
        let v = vectors();
        let n_cent = crate::corpus::n_cent(N_DOCS) as u32;

        let mut g = c.benchmark_group("supertable_vec_build");
        g.sample_size(10);
        g.throughput(Throughput::Elements(N_DOCS as u64));

        g.bench_function(
            format!("supertable_{N_DOCS}docs_{N_SEGMENTS}superfiles"),
            |b| {
                b.iter_with_large_drop(|| {
                    let _ = black_box(v);
                    build_supertable()
                });
            },
        );

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
            if let Some(c_st) = cal.supertable[i] {
                g.bench_with_input(
                    BenchmarkId::new(
                        format!("supertable_{label}"),
                        format!("p={},r={}", c_st.probe, c_st.refine),
                    ),
                    &(c_st.probe, c_st.refine),
                    |b, &(p, r)| {
                        let q = &qs[0];
                        let opts = VectorSearchOptions::new()
                            .with_nprobe(p)
                            .with_rerank_mult(r);
                        b.iter(|| {
                            let hits = supertable_topk(st, black_box(q), TOP_K, opts);
                            black_box(hits)
                        });
                    },
                );
            }
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
                            let hits = crate::lance::search_lance(&lh.rt, &lh.table, q, TOP_K, p, r);
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
    use crate::markdown::{MarkdownSection, fmt_throughput, fmt_time, fmt_winner, read_mean_ns};

    let group = "supertable_vec_build";
    let st_bench = format!("supertable_{N_DOCS}docs_{N_SEGMENTS}superfiles");
    let lance_bench = format!("lance_{N_DOCS}docs");
    let st_ns = read_mean_ns(group, &st_bench);
    let lance_ns = read_mean_ns(group, &lance_bench);

    let mut body = String::new();
    body.push_str(&format!(
        "### Supertable vector — ingest ({N_DOCS} docs × dim={}, sharded into {N_SEGMENTS} superfiles)\n\n",
        crate::lance::DIM
    ));
    body.push_str("| Engine | Time | Throughput | vs LanceDB |\n");
    body.push_str("|--------|------|------------|------------|\n");
    for (label, ns, baseline, is_baseline) in [
        ("supertable", st_ns, lance_ns, false),
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
    use crate::markdown::{MarkdownSection, fmt_time, fmt_winner, read_mean_ns};

    let cal = calibrations();
    let group = "supertable_vec_search";

    let mut body = String::new();
    body.push_str(&format!(
        "### Supertable vector — search ({N_DOCS} docs × dim={}, calibrated at recall targets)\n\n",
        crate::lance::DIM
    ));
    body.push_str("| Recall target | supertable (probe/seg, refine) | supertable p50 | Lance (probe, refine) | Lance p50 | Winner |\n");
    body.push_str("|---------------|--------------------------------|----------------|-----------------------|-----------|--------|\n");

    for (i, &target) in RECALL_TARGETS.iter().enumerate() {
        let label = format!("recall_at_least_{:02}", (target * 100.0) as u32);
        let (st_cell, st_ns) = match cal.supertable[i] {
            Some(c) => {
                let bid = format!("supertable_{label}/p={},r={}", c.probe, c.refine);
                let ns = read_mean_ns(group, &bid);
                (format!("(p={}, r={})", c.probe, c.refine), ns)
            }
            None => ("—".into(), None),
        };
        let (lan_cell, lan_ns) = match cal.lance[i] {
            Some(c) => {
                let r_u32 = c.refine as u32;
                let bid = format!("lance_{label}/p={},r={}", c.probe, r_u32);
                let ns = read_mean_ns(group, &bid);
                (format!("(p={}, r={})", c.probe, r_u32), ns)
            }
            None => ("—".into(), None),
        };
        let st_t = st_ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let lan_t = lan_ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let winner = fmt_winner("supertable", st_ns, "lance", lan_ns);
        body.push_str(&format!(
            "| {:.2} | {} | {} | {} | {} | {} |\n",
            target, st_cell, st_t, lan_cell, lan_t, winner
        ));
    }

    crate::markdown::emit(&MarkdownSection {
        anchor_id: "bench/vector/supertable/search".into(),
        body,
    });
}

criterion_group!(benches, bench);
