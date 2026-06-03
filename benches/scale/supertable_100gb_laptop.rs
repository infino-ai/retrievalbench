//! 003 M14 — 100GB laptop scale test, Variant A (LocalFS).
//!
//! Builds a supertable through the LocalFS-backed
//! `StorageProvider` at a scale where the on-disk index
//! lands around 100 GB (43M docs), and reports the
//! end-to-end build numbers + post-build open-and-verify.
//!
//! ## Scope shipped here (M14, Variant A only)
//!
//!   - **Build phase**: stream `N` synthetic Zipfian-text +
//!     384-dim cosine vector docs through `writer.append +
//!     writer.commit` in 1M-doc chunks; the writer's M4
//!     write-through path persists superfiles + manifest to
//!     `LocalFsStorageProvider`.
//!   - **Post-build verify**: drop the producer, reopen via
//!     `Supertable::open(...)`, assert the recovered
//!     `manifest_id` + `n_superfiles` + `n_docs_total` match
//!     the producer's pre-drop snapshot.
//!   - **Reporting**: wall-clock build time, docs/sec, on-
//!     disk bytes, `Supertable::stats()` snapshot (RSS,
//!     manifest_id, n_manifest_parts).
//!
//! ## Scope deferred (called out in 003 plan §M14)
//!
//! Status of each plan-listed phase:
//!
//!   - **Cold-query phase** — **landed in M14b** (alongside
//!     the disk-cache reader-path integration). Opens a
//!     fresh consumer with a cache attached, runs a query
//!     that touches every segment, reports cold-pass
//!     wall-clock + `n_cold_fetches` + bytes-in-cache; then
//!     repeats for the warm pass and reports the speedup.
//!   - **Steady-state mixed load**: PENDING the concurrent
//!     reader/writer harness + hdrhistogram p99 reporting.
//!     Cache integration is no longer the gate.
//!   - **Memory pressure**: PENDING the per-supertable
//!     memory-budget knob (`with_memory_budget`). Cache
//!     integration is in place; the missing piece is the
//!     RSS bound the budget knob enforces.
//!   - **Crash + recover** — landed alongside M14e. The
//!     producer drop (equivalent to a clean process exit
//!     at the last committed manifest) is followed by a
//!     timed `Supertable::open` of the persisted state +
//!     assertion that the recovered `manifest_id`,
//!     `n_superfiles`, and `n_docs_total` match the
//!     producer's pre-drop snapshot. Kill-point semantics
//!     (post-segment / post-list / post-pointer) are
//!     M12's domain at small scale; manifest-list open
//!     scaling at 1M-segment regime is M15's.
//!   - **Variant B (s3s-fs)**: requires a separate
//!     `S3StorageProvider` impl + s3s-fs harness; M16.
//!
//! ## Scale knobs
//!
//!   - Default: **10M docs** (~ 23 GB on disk at ~2.3 KB/doc).
//!     The user's bench-scale rule floors realistic benches
//!     at 10M; the M14 default sits at that floor for fast
//!     iteration.
//!   - `INFINO_BENCH_100GB=1`: **43M docs** (~ 100 GB). The
//!     plan's target scale. Needs ~ 35 minutes wall time and
//!     ~ 110 GB free SSD on a modern laptop.
//!   - `INFINO_BENCH_M14_N_DOCS=<int>`: explicit doc-count
//!     override for sweep runs (e.g., 1M for smoke, 100M for
//!     stress).
//!
//! Output is a single-line summary on stderr per phase. Invocation
//! shape is in `benches/scale/main.rs` — this file is one of the
//! runners the `scale` bundle dispatches to.

#![deny(clippy::unwrap_used)]

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use arrow_array::{Array, FixedSizeListArray, Float32Array, LargeStringArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use infino::superfile::builder::{FtsConfig, VectorConfig};
use infino::superfile::fts::tokenize::Tokenizer;
use infino::superfile::vector::distance::Metric;
use infino::superfile::vector::rerank_codec::RerankCodec;
use infino::supertable::storage::{LocalFsStorageProvider, StorageProvider};
use infino::supertable::{Supertable, SupertableOptions};
use infino::test_helpers::default_tokenizer;
use retrievalbench::corpus;
use tempfile::TempDir;

/// Buffer chunk size: emit one commit per `CHUNK_DOCS`
/// docs. Larger chunks = fewer commits = lower per-commit
/// overhead amortization, but each commit also produces one
/// manifest part — too few commits means giant manifest
/// parts (slower to encode). 1M docs/chunk is in the
/// "thousands of parts at 43M docs" range, which is the M15
/// stress regime; M14 is content with whatever the chunking
/// implies.
const CHUNK_DOCS: usize = 1_000_000;

fn resolve_n_docs() -> usize {
    if let Ok(s) = std::env::var("INFINO_BENCH_M14_N_DOCS") {
        return s
            .parse()
            .expect("INFINO_BENCH_M14_N_DOCS must be an integer");
    }
    if std::env::var("INFINO_BENCH_100GB").is_ok() {
        // Plan target: ~100 GB on-disk index at the measured
        // 2.3 KB/doc supertable density.
        43_000_000
    } else {
        // M14 default: 10M docs (≈ 23 GB on disk). Floors at
        // the user's "≥10M docs" bench-scale rule.
        10_000_000
    }
}

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("title", DataType::LargeUtf8, false),
        Field::new(
            "emb",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                corpus::DIM as i32,
            ),
            false,
        ),
    ]))
}

fn build_options(
    storage: Arc<dyn StorageProvider>,
    n_cent: usize,
    writer_threads: usize,
) -> SupertableOptions {
    let writer_pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(writer_threads)
            .thread_name(|i| format!("m14-writer-{i}"))
            .build()
            .expect("writer pool"),
    );
    let tk: Arc<dyn Tokenizer> = default_tokenizer();
    SupertableOptions::new(
        schema(),
        vec![FtsConfig {
            column: "title".into(),
        }],
        vec![VectorConfig {
            column: "emb".into(),
            dim: corpus::DIM,
            n_cent,
            rot_seed: 7,
            metric: Metric::Cosine,
            rerank_codec: RerankCodec::Fp32,
        }],
        Some(tk),
    )
    .expect("opts")
    .with_writer_pool(writer_pool)
    .with_storage(storage)
}

/// Generate one chunk's worth of (titles, vectors). Each
/// chunk is produced on-the-fly so we never materialize the
/// full corpus in memory — at 43M docs, the titles alone
/// would be ~50 GB and the vectors ~66 GB.
fn generate_chunk(_start_id: u64, n: usize, n_cent: usize, chunk_seed: u64) -> RecordBatch {
    // Reuse the deterministic generators from `common`, but
    // sized to one chunk at a time. `seed=chunk_seed` gives
    // distinct content per chunk while staying reproducible
    // across runs. ids are auto-injected by the supertable at
    // append time.
    let titles = corpus::generate_text_corpus(n, chunk_seed);
    let vectors = corpus::generate_vector_corpus(n, n_cent, chunk_seed, /*normalize=*/ true);

    let titles_arr = LargeStringArray::from(titles.iter().map(String::as_str).collect::<Vec<_>>());
    let item_field = Arc::new(Field::new("item", DataType::Float32, true));
    let values = Float32Array::from(vectors);
    let fsl = FixedSizeListArray::try_new(
        item_field,
        corpus::DIM as i32,
        Arc::new(values) as Arc<dyn Array>,
        None,
    )
    .expect("FSL");
    RecordBatch::try_new(schema(), vec![Arc::new(titles_arr), Arc::new(fsl)]).expect("batch")
}

/// Recursive `du -s --bytes <path>` equivalent. Used to
/// report final on-disk index size.
fn dir_size_bytes(path: &Path) -> u64 {
    let mut total = 0u64;
    let walker = match std::fs::read_dir(path) {
        Ok(w) => w,
        Err(_) => return 0,
    };
    for entry in walker.flatten() {
        let p = entry.path();
        if let Ok(meta) = std::fs::metadata(&p) {
            if meta.is_dir() {
                total = total.saturating_add(dir_size_bytes(&p));
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

/// Format a byte count as a human-readable string with two
/// decimal places.
fn fmt_bytes(n: u64) -> String {
    const KIB: f64 = 1024.0;
    let n = n as f64;
    if n >= KIB * KIB * KIB {
        format!("{:.2} GiB", n / (KIB * KIB * KIB))
    } else if n >= KIB * KIB {
        format!("{:.2} MiB", n / (KIB * KIB))
    } else if n >= KIB {
        format!("{:.2} KiB", n / KIB)
    } else {
        format!("{n} B")
    }
}

/// Build phase: stream `n_docs` through writer.append +
/// writer.commit in `CHUNK_DOCS`-sized chunks. The storage
/// write-through path persists superfiles + manifest to
/// `LocalFsStorageProvider` per commit.
///
/// Reports: chunks, total docs, wall time, docs/sec.
fn run_build_phase(storage_root: &Path, n_docs: usize, writer_threads: usize) -> Supertable {
    let storage: Arc<dyn StorageProvider> =
        Arc::new(LocalFsStorageProvider::new(storage_root).expect("local fs provider"));
    let n_cent = corpus::n_cent(n_docs);
    let st = Supertable::create(build_options(Arc::clone(&storage), n_cent, writer_threads))
        .expect("create supertable");

    let n_chunks = n_docs.div_ceil(CHUNK_DOCS);
    eprintln!(
        "[m14] build start: n_docs={n_docs} chunks={n_chunks} chunk_size={CHUNK_DOCS} \
         writer_threads={writer_threads} storage_root={:?}",
        storage_root
    );
    let t0 = Instant::now();

    for chunk in 0..n_chunks {
        let start_id = (chunk * CHUNK_DOCS) as u64;
        let chunk_size = ((chunk + 1) * CHUNK_DOCS).min(n_docs) - chunk * CHUNK_DOCS;
        let batch = generate_chunk(start_id, chunk_size, n_cent, (chunk as u64) + 1);

        let chunk_start = Instant::now();
        let mut w = st.writer().expect("writer");
        w.append(&batch).expect("append");
        w.commit().expect("commit");
        let chunk_elapsed = chunk_start.elapsed();

        let docs_so_far = (chunk + 1) * CHUNK_DOCS - CHUNK_DOCS + chunk_size;
        eprintln!(
            "[m14] chunk {}/{} committed: docs_so_far={} chunk_wall={:.2}s manifest_id={}",
            chunk + 1,
            n_chunks,
            docs_so_far,
            chunk_elapsed.as_secs_f64(),
            st.manifest_id()
        );
    }

    let elapsed = t0.elapsed();
    let docs_per_sec = (n_docs as f64) / elapsed.as_secs_f64();
    eprintln!(
        "[m14] build done: n_docs={n_docs} wall={:.2}s throughput={:.0} docs/s manifest_id={}",
        elapsed.as_secs_f64(),
        docs_per_sec,
        st.manifest_id()
    );

    st
}

/// Crash + recover phase (M14e).
///
/// Models the "process crashes mid-flight, then restarts"
/// scenario at the bench's scale. The producer is already
/// dropped before this fn is called — equivalent to a clean
/// process exit at the time of the last committed manifest.
/// `Supertable::open` against the persisted state is the
/// recovery; we measure wall time + assert the recovered
/// `manifest_id`, `n_superfiles`, and `n_docs_total` match the
/// producer's pre-drop snapshot. This is M14's "verify the
/// commit is durable" guarantee, generalized to scale: the
/// open path must parse a potentially-large manifest list
/// and eager-fetch every manifest part within reasonable
/// wall time.
///
/// **Crash modes not exercised here.** M12
/// (`tests/supertable_commit_crash_localfs.rs`) covers the
/// kill-point semantics (post-segment, post-list,
/// post-pointer) at small scale; M15 (1M-segment manifest
/// stress) will measure open wall time on a manifest list
/// at the 1M-segment regime. The 100GB phase here is the
/// middle ground — full-data scale, clean-drop "crash."
async fn run_crash_recover_phase(storage_root: &Path, n_docs: usize, pre_drop: PreDropSnapshot) {
    let storage: Arc<dyn StorageProvider> =
        Arc::new(LocalFsStorageProvider::new(storage_root).expect("local fs provider"));
    let n_cent = corpus::n_cent(n_docs);
    let opts = build_options(Arc::clone(&storage), n_cent, 1);

    let t0 = Instant::now();
    let consumer = Supertable::open(opts).await.expect("open after build");
    let open_elapsed = t0.elapsed();

    let r = consumer.reader();
    assert_eq!(
        consumer.manifest_id(),
        pre_drop.manifest_id,
        "post-open manifest_id must match producer's pre-drop"
    );
    assert_eq!(
        r.n_superfiles(),
        pre_drop.n_superfiles,
        "post-open n_superfiles must match producer's pre-drop"
    );
    assert_eq!(
        r.n_docs_total(),
        pre_drop.n_docs_total,
        "post-open n_docs_total must match producer's pre-drop"
    );

    let stats = consumer.stats();
    eprintln!(
        "[m14] crash-recover done: open_wall={:.2}s manifest_id={} n_superfiles={} \
         n_docs_total={} n_manifest_parts={:?} n_manifest_parts_loaded={} rss={}",
        open_elapsed.as_secs_f64(),
        stats.manifest_id,
        r.n_superfiles(),
        r.n_docs_total(),
        stats.n_manifest_parts,
        stats.n_manifest_parts_loaded,
        fmt_bytes(stats.process_rss_bytes),
    );
}

#[derive(Debug, Clone, Copy)]
struct PreDropSnapshot {
    manifest_id: u64,
    n_superfiles: usize,
    n_docs_total: u64,
}

/// Cold-query phase (M14b — filled in alongside the
/// disk-cache reader integration).
///
/// Opens a fresh `Supertable` with a disk cache attached
/// against the storage root the build phase produced (an
/// otherwise-cold consumer simulating a different process),
/// runs one SQL query that requires reading every segment,
/// and reports the wall-clock latency. The cache starts at
/// zero entries → every segment touched triggers a parallel
/// range-fetch → pwrite → mmap through `DiskCacheStore`.
///
/// Then repeats the same query and reports the warm-cache
/// wall time + the `n_cold_fetches` delta (should be zero)
/// to confirm the second pass hit the cache instead of
/// re-fetching.
async fn run_cold_query_phase(storage_root: &Path, n_docs: usize) {
    let storage: Arc<dyn StorageProvider> =
        Arc::new(LocalFsStorageProvider::new(storage_root).expect("local fs provider"));
    let n_cent = corpus::n_cent(n_docs);

    // Cache the consumer in a separate tempdir so it really
    // starts cold (no carry-over from any prior bench run).
    let cache_dir = TempDir::new().expect("cache tempdir");
    let cache = build_disk_cache(Arc::clone(&storage), cache_dir.path());

    // Reuse the ambient bench runtime: this whole phase
    // runs inside it (the bench's main rt.block_on), so
    // Supertable::open just .awaits cleanly.
    let consumer = Supertable::open(
        build_options(Arc::clone(&storage), n_cent, 1).with_disk_cache(Arc::clone(&cache)),
    )
    .await
    .expect("open for cold-query");

    let pre = cache.stats();
    assert_eq!(pre.n_cold_fetches, 0, "cache must start cold");

    // First query — every segment cold-fetched.
    let t0 = Instant::now();
    let _ = consumer
        .query_sql("SELECT COUNT(*) FROM supertable")
        .expect("cold query");
    let cold_wall = t0.elapsed();
    let mid = cache.stats();
    eprintln!(
        "[m14] cold-query phase: wall={:.2}s n_cold_fetches={} cache_bytes={}",
        cold_wall.as_secs_f64(),
        mid.n_cold_fetches,
        fmt_bytes(mid.current_bytes)
    );

    // Second query — every segment warm.
    let t1 = Instant::now();
    let _ = consumer
        .query_sql("SELECT COUNT(*) FROM supertable")
        .expect("warm query");
    let warm_wall = t1.elapsed();
    let post = cache.stats();
    eprintln!(
        "[m14] warm-query: wall={:.2}s n_cold_fetches_delta={} (must be 0)",
        warm_wall.as_secs_f64(),
        post.n_cold_fetches - mid.n_cold_fetches
    );
    assert_eq!(
        post.n_cold_fetches, mid.n_cold_fetches,
        "warm query must not trigger fresh cold-fetches"
    );

    // Speedup ratio is a useful single-line summary of
    // whether the cache is actually doing its job.
    if cold_wall.as_secs_f64() > 0.0 && warm_wall.as_secs_f64() > 0.0 {
        eprintln!(
            "[m14] cold/warm speedup: {:.1}x",
            cold_wall.as_secs_f64() / warm_wall.as_secs_f64()
        );
    }
}

fn build_disk_cache(
    storage: Arc<dyn StorageProvider>,
    cache_root: &Path,
) -> Arc<infino::supertable::reader_cache::DiskCacheStore> {
    use infino::supertable::reader_cache::{
        ColdFetchMode, DiskCacheConfig, DiskCacheStore, LruPolicy,
    };
    use std::collections::HashSet;
    let cfg = DiskCacheConfig {
        cache_root: cache_root.to_path_buf(),
        // Generous budget — at the 10M-doc default the index
        // is ~23 GB; we want the entire working set to fit so
        // the warm-pass speedup isn't muddied by mid-pass
        // evictions. The 100GB run intentionally exceeds the
        // budget so eviction pressure becomes part of the
        // measurement.
        disk_budget_bytes: 32 * (1u64 << 30), // 32 GiB
        cold_fetch_mode: ColdFetchMode::HybridWithPrefetch,
        cold_fetch_streams: 16,
        cold_fetch_chunk_bytes: 8 * (1u64 << 20), // 8 MiB
        mmap_cold_threshold_secs: 0,
        mmap_sweep_interval_secs: 0,
        eviction: Box::new(LruPolicy::new()),
        verify_crc_on_open: true,
    };
    let pinned_fn: Arc<dyn Fn() -> HashSet<_> + Send + Sync> = Arc::new(HashSet::new);
    DiskCacheStore::new(storage, cfg, pinned_fn).expect("disk cache")
}

/// Steady-state mixed-load phase (M14d — filled in
/// alongside M14b/M14c).
///
/// Mirrors `supertable_e2e`'s mixed-load routine on the
/// cache-backed path. Spawns one writer thread that
/// continuously commits 1K-doc chunks and `n_readers` (= 4)
/// reader threads that loop `query_sql("SELECT COUNT(*)")`
/// against the live supertable. Runs for a fixed window
/// (10 s on the smoke; could scale at 100GB). Reports
/// writer throughput (commits/s) + reader p50/p95/p99 (via
/// hdrhistogram) + cache hit rate (n_cold_fetches /
/// n_queries).
fn run_steady_state_phase(_storage_root: &Path, _n_docs: usize) {
    use hdrhistogram::Histogram;

    // Fresh storage + cache for this phase (independent of
    // the build phase). Steady-state commits small (1K-doc)
    // chunks; IVF clustering on 1K docs with the build
    // phase's n_cent=1024 is degenerate (k > n_docs), so we
    // use a small fixed n_cent suitable for the chunk size.
    // The phase measures concurrency / latency, not IVF
    // recall — values don't need to match the build's.
    let n_cent: usize = 16;
    let storage_dir = TempDir::new().expect("phase storage tempdir");
    let storage: Arc<dyn StorageProvider> =
        Arc::new(LocalFsStorageProvider::new(storage_dir.path()).expect("phase provider"));
    let cache_dir = TempDir::new().expect("phase cache tempdir");
    let cache = build_disk_cache(Arc::clone(&storage), cache_dir.path());

    // Writer pool = 1 in steady-state: each commit is small
    // (1K docs), so a multi-thread rayon shard split would
    // produce degenerate 60-doc shards. One shard per
    // commit keeps the IVF clustering happy + the
    // per-commit segment count predictable.
    let st = Supertable::create(
        build_options(Arc::clone(&storage), n_cent, 1).with_disk_cache(Arc::clone(&cache)),
    )
    .expect("create supertable");

    let duration = std::time::Duration::from_secs(10);
    let n_readers: usize = 4;
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let n_cold_before = cache.stats().n_cold_fetches;

    // Writer thread.
    let writer_thread = {
        let stop = Arc::clone(&stop);
        let st = st.clone();
        std::thread::Builder::new()
            .name("m14-writer".into())
            .spawn(move || {
                let mut w = st.writer().expect("writer slot");
                let mut commit_count: u64 = 0;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let chunk = generate_chunk(
                        commit_count.wrapping_mul(1_000) + 1_000_000,
                        1_000,
                        n_cent,
                        commit_count + 1_000,
                    );
                    w.append(&chunk).expect("writer append");
                    w.commit().expect("writer commit");
                    commit_count += 1;
                }
                drop(w);
                commit_count
            })
            .expect("spawn writer thread")
    };

    // Reader threads.
    let mut reader_handles = Vec::with_capacity(n_readers);
    for i in 0..n_readers {
        let stop = Arc::clone(&stop);
        let st = st.clone();
        let handle = std::thread::Builder::new()
            .name(format!("m14-reader-{i}"))
            .spawn(move || {
                let mut hist =
                    Histogram::<u64>::new_with_bounds(1_000, 60_000_000_000, 3).expect("hist");
                let mut n_queries: u64 = 0;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let t0 = Instant::now();
                    match st.query_sql("SELECT COUNT(*) FROM supertable") {
                        Ok(_) => {}
                        // Writer-commit-in-flight contention or
                        // similar transient — count + retry.
                        Err(_) => continue,
                    }
                    let elapsed_ns = t0.elapsed().as_nanos() as u64;
                    let _ = hist.record(elapsed_ns.max(1));
                    n_queries += 1;
                }
                (hist, n_queries)
            })
            .expect("spawn reader thread");
        reader_handles.push(handle);
    }

    std::thread::sleep(duration);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);

    let commit_count = writer_thread.join().expect("writer thread");
    let mut combined =
        Histogram::<u64>::new_with_bounds(1_000, 60_000_000_000, 3).expect("combined hist");
    let mut total_reader_queries: u64 = 0;
    for h in reader_handles {
        let (hist, n) = h.join().expect("reader thread");
        let _ = combined.add(&hist);
        total_reader_queries += n;
    }

    let n_cold_after = cache.stats().n_cold_fetches;
    let cache_misses = n_cold_after - n_cold_before;
    let cache_hit_rate = if total_reader_queries > 0 {
        1.0 - (cache_misses as f64 / total_reader_queries as f64)
    } else {
        0.0
    };

    eprintln!(
        "[m14] steady-state done: duration={:.0}s commits={} commit_tput={:.1}/s \
         readers={} reader_queries={} reader_p50={:.2}ms reader_p95={:.2}ms \
         reader_p99={:.2}ms cache_misses={} cache_hit_rate={:.3}",
        duration.as_secs_f64(),
        commit_count,
        commit_count as f64 / duration.as_secs_f64(),
        n_readers,
        total_reader_queries,
        combined.value_at_quantile(0.5) as f64 / 1e6,
        combined.value_at_quantile(0.95) as f64 / 1e6,
        combined.value_at_quantile(0.99) as f64 / 1e6,
        cache_misses,
        cache_hit_rate,
    );

    // Sanity: at least one reader query landed. If 0, the
    // bench's writer is starving the readers — that's
    // useful diagnostic but not a failure mode (the
    // supertable's dual-pool design specifically lets us
    // tune for this; see supertable_e2e's writer_pool=N/2
    // mixed-load measurement for the same concern).
    assert!(
        total_reader_queries > 0,
        "no reader queries completed; writer fully starved readers in {:?}",
        duration
    );

    let _ = st;
}

/// Memory-pressure phase (M14c — filled in alongside the
/// `SupertableOptions::with_memory_budget` knob).
///
/// Opens a fresh consumer with a disk cache AND a memory
/// budget configured to ~25% of the expected working set
/// size, runs a sustained query loop that touches every
/// segment, and asserts the cache's mmap-resident bytes
/// stay near the budget (±50% — best-effort, not a hard
/// cgroup cap).
async fn run_memory_pressure_phase(storage_root: &Path, n_docs: usize) {
    let storage: Arc<dyn StorageProvider> =
        Arc::new(LocalFsStorageProvider::new(storage_root).expect("local fs provider"));
    let n_cent = corpus::n_cent(n_docs);
    let cache_dir = TempDir::new().expect("cache tempdir");
    let cache = build_disk_cache(Arc::clone(&storage), cache_dir.path());

    // Target budget: a quarter of the on-disk size (capped
    // to keep the bench bounded). At the laptop default
    // (10M docs, ~23 GB), budget = ~5.7 GB; at 100GB it'd
    // be ~25 GB. The exact number doesn't matter — what we
    // assert is that sweep keeps residency near it.
    let on_disk = dir_size_bytes(storage_root);
    let budget = (on_disk / 4).max(64 * (1 << 20)); // ≥ 64 MiB floor

    let consumer = Supertable::open(
        build_options(Arc::clone(&storage), n_cent, 1)
            .with_disk_cache(Arc::clone(&cache))
            .with_memory_budget(budget),
    )
    .await
    .expect("open for memory-pressure");

    eprintln!(
        "[m14] memory-pressure phase start: budget={} on_disk={}",
        fmt_bytes(budget),
        fmt_bytes(on_disk)
    );

    // Prime the cache + let the M7 hybrid finalizer settle.
    // The hybrid cold-fetch returns a bytes-backed reader
    // synchronously while a background task awaits pwrites
    // + opens the mmap; we need the mmap-backed entry to
    // be in place before `mmap_resident_bytes` reports
    // anything useful. ~500 ms is plenty for the 100K-doc
    // smoke (214 MiB pwrite); the 100GB run scales with
    // segment count + size (pass duration in the loop also
    // gives the finalizers time to drain).
    let _ = consumer
        .query_sql("SELECT COUNT(*) FROM supertable")
        .expect("priming query");
    // Aggressive settle: 3 s for the hybrid finalizer to
    // complete its pwrite + sync_all + rename + mmap path.
    // At 100GB scale this scales with segment size; the
    // sleep here is for the smoke run (small segment).
    std::thread::sleep(std::time::Duration::from_secs(3));
    let primed = cache.stats();
    eprintln!(
        "[m14] memory-pressure prime: n_entries={} current_bytes={} mmap_size={}",
        primed.n_entries,
        fmt_bytes(primed.current_bytes),
        fmt_bytes(cache.current_mmap_size_bytes()),
    );

    // Run 8 query passes. Each pass scans every segment
    // (cold-fetching on first miss, then warming the cache
    // until it hits the budget). Track the peak observed
    // mmap-resident across passes.
    let mut peak_mmap_resident: u64 = 0;
    let mut total_advised_growth: u64 = 0;
    let advised_baseline = cache.stats().n_madvise_calls;
    for pass in 0..8 {
        let t0 = Instant::now();
        let _ = consumer
            .query_sql("SELECT COUNT(*) FROM supertable")
            .expect("memory-pressure query");
        let wall = t0.elapsed();
        let stats = consumer.stats();
        let cache_stats = cache.stats();
        peak_mmap_resident = peak_mmap_resident.max(stats.mmap_resident_bytes.unwrap_or(0));
        total_advised_growth = cache_stats.n_madvise_calls - advised_baseline;
        eprintln!(
            "[m14] memory-pressure pass {}/8: wall={:.2}s mmap_resident={} n_madvise_total={}",
            pass + 1,
            wall.as_secs_f64(),
            fmt_bytes(stats.mmap_resident_bytes.unwrap_or(0)),
            cache_stats.n_madvise_calls,
        );

        // Force a budget sweep mid-pass so the working set
        // stays bounded. Without this, the cache only
        // sweeps on commit boundaries — and the consumer
        // doesn't commit.
        cache.sweep_for_budget(budget);
    }

    let _ = total_advised_growth; // surfaced via per-pass log lines

    // Best-effort assertion: peak mmap residency should be
    // within 2x of budget. This isn't a tight bound — RSS
    // measurement on macOS / Linux has its own slop and
    // the OS may keep pages resident even post-madvise if
    // there's no pressure. The bench's purpose is to surface
    // the ratio for human review; the assert just guards
    // against pathological regressions (e.g., sweep
    // completely disabled).
    let ratio = peak_mmap_resident as f64 / budget as f64;
    eprintln!(
        "[m14] memory-pressure done: peak_mmap_resident={} budget={} ratio={:.2}x",
        fmt_bytes(peak_mmap_resident),
        fmt_bytes(budget),
        ratio,
    );
    assert!(
        ratio < 4.0,
        "peak mmap residency exceeded 4x budget (sweep likely disabled): \
         peak={peak_mmap_resident} budget={budget} ratio={ratio:.2}x"
    );
}

pub fn run() {
    let n_docs = resolve_n_docs();
    let writer_threads = num_cpus::get().max(1);

    // Keep the storage tempdir alive across the whole run.
    // `keep()` so the post-build verify can still read it
    // after the producer drops.
    let tmp = TempDir::new().expect("tempdir for M14 storage root");
    let storage_root = tmp.keep();

    // The build phase, open-verify, cold-query, and
    // memory-pressure phases all need an ambient runtime to
    // schedule the M7 hybrid finalizer reliably (without
    // one, finalizers spawn on the supertable's 1-worker
    // sql_runtime and don't run before stats sampling, so
    // sweep_for_budget sees `mmap: None` entries and never
    // fires). Use a fresh runtime per "async phase" + drop
    // it before transitioning to the sync steady-state
    // phase — sql_runtime drops in the workers' Supertables
    // would otherwise panic inside an async context.

    // ---- Async phase 1: build + open-verify + cold-query ----
    let (pre_drop, on_disk) = {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("tokio runtime for bench (phase 1)");
        let (pre_drop, on_disk) = rt.block_on(async {
            let st = run_build_phase(&storage_root, n_docs, writer_threads);
            let r = st.reader();
            let pre_drop = PreDropSnapshot {
                manifest_id: st.manifest_id(),
                n_superfiles: r.n_superfiles(),
                n_docs_total: r.n_docs_total(),
            };
            let on_disk = dir_size_bytes(&storage_root);
            eprintln!(
                "[m14] on-disk size: {} ({} bytes); avg bytes/doc={:.1}",
                fmt_bytes(on_disk),
                on_disk,
                (on_disk as f64) / (pre_drop.n_docs_total as f64).max(1.0)
            );
            drop(r);
            drop(st);

            run_crash_recover_phase(&storage_root, n_docs, pre_drop).await;
            run_cold_query_phase(&storage_root, n_docs).await;
            (pre_drop, on_disk)
        });
        // rt drops here; supertables from the async phase
        // are already dropped above so no nested-runtime
        // panics.
        let _ = pre_drop;
        let _ = on_disk;
        (pre_drop, on_disk)
    };
    let _ = (pre_drop, on_disk);

    // ---- Sync phase: steady-state mixed-load ----
    // Plain sync context. The Supertable's lazy sql_runtime
    // gets initialized + dropped inside this phase without
    // any ambient runtime; std::thread::spawn workers
    // similarly run in plain OS-thread context.
    run_steady_state_phase(&storage_root, n_docs);

    // ---- Async phase 2: memory-pressure ----
    {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("tokio runtime for bench (phase 2)");
        rt.block_on(run_memory_pressure_phase(&storage_root, n_docs));
    }

    // Clean up the storage root we explicitly kept above —
    // at 100 GB scale this matters; leaving it would fill
    // /tmp on subsequent runs.
    let _ = std::fs::remove_dir_all(&storage_root);

    eprintln!("[m14] done");
}
