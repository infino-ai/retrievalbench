//! Reader-p99-under-writer-load mixed-load epilogue.
//!
//! Bespoke (non-criterion) routine: runs N reader queries
//! concurrent with a long-running commit twice — once with
//! `writer_pool = num_cpus()` (CPU-saturating extreme) and once
//! with `writer_pool = max(1, num_cpus() / 2)` (default isolation
//! shape). Reports both p99s as a single line per configuration
//! plus the saturating/isolated ratio. Validates the dual-pool
//! design's "reader p99 stable under writer load" claim.
//!
//! Wraps in a no-op `criterion_group` so it slots into the bundle's
//! `criterion_main!`; the actual work prints to stderr.

use crate::common::*;
use criterion::{Criterion, criterion_group};
use hdrhistogram::Histogram;
use infino::superfile::fts::reader::BoolMode;
use infino::superfile::vector::distance::normalize;
use infino::supertable::query::vector::VectorSearchOptions;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand_distr::{Distribution, StandardNormal};
use retrievalbench::corpus;
use std::sync::{Arc, atomic::AtomicBool};
use std::thread;
use std::time::{Duration, Instant};

/// Long-running commit's worth of work: rebuild the supertable
/// from scratch in a background thread, while the foreground
/// runs `n_queries` searches against a pre-pinned reader. Records
/// foreground query latencies into a hdrhistogram and returns the
/// p99 in nanoseconds.
fn mixed_load_p99_ns(writer_threads: usize, n_queries: usize) -> u64 {
    let corpus = corpus();
    let st = build_supertable(corpus, writer_threads);

    // Pin a reader before the background commit storm starts.
    // Concurrent commits won't perturb this snapshot — the ArcSwap
    // guarantee — so the bench measures *contention*, not visibility
    // drift.
    let r = st.reader();

    // Background thread: hammer commits on a separate Supertable
    // (writer slot is exclusive per Supertable, so we can't share
    // st's writer). The contention point is the rayon writer pool's
    // CPU-time budget vs the foreground reader-pool queries.
    let st_for_writer = build_supertable(corpus, writer_threads);
    let stop_flag = Arc::new(AtomicBool::new(false));
    let writer_thread = {
        let stop = Arc::clone(&stop_flag);
        let chunk = batches()[0].clone();
        thread::spawn(move || {
            let mut w = st_for_writer.writer().expect("writer");
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                if w.append(&chunk).is_err() {
                    break;
                }
                if w.commit().is_err() {
                    break;
                }
            }
            drop(w);
        })
    };

    // Build query inputs once.
    let mut rng = StdRng::seed_from_u64(202);
    let dist = StandardNormal;
    let mut q: Vec<f32> = (0..corpus::DIM)
        .map(|_| {
            let s: f64 = dist.sample(&mut rng);
            (s as f32) * 3.0
        })
        .collect();
    normalize(&mut q);

    // Foreground: time each query, record into hdrhistogram. Three
    // buckets per round to vary the work shape.
    let mut hist =
        Histogram::<u64>::new_with_bounds(1_000, 60_000_000_000, 3).expect("hdr histogram");
    for i in 0..n_queries {
        let start = Instant::now();
        match i % 3 {
            0 => {
                let _hits = retrievalbench::corpus::block_on_inmem(r.bm25_search(
                    "title",
                    "term00001",
                    TOP_K,
                    BoolMode::Or,
                ))
                .expect("bm25");
            }
            1 => {
                let _hits = retrievalbench::corpus::block_on_inmem(
                    r.bm25_search_prefix("title", "term000", TOP_K),
                )
                .expect("bm25_prefix");
            }
            _ => {
                let _hits = retrievalbench::corpus::block_on_inmem(r.vector_search(
                    "emb",
                    &q,
                    TOP_K,
                    VectorSearchOptions::new(),
                ))
                .expect("vector");
            }
        }
        let elapsed_ns = start.elapsed().as_nanos();
        let v = u64::try_from(elapsed_ns).unwrap_or(u64::MAX);
        let _ = hist.record(v);
    }

    stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
    writer_thread.join().expect("writer thread");

    hist.value_at_quantile(0.99)
}

fn bench_mixed_load(_c: &mut Criterion) {
    let n_cpus = num_cpus::get().max(1);
    let n_queries = if std::env::var("INFINO_BENCH_FULL").is_ok() {
        2_000
    } else {
        500
    };

    eprintln!("[hybrid_mixed_load] reader p99 under writer load (n_queries={n_queries}):");
    let p99_saturating = mixed_load_p99_ns(n_cpus, n_queries);
    eprintln!(
        "  writer_pool=num_cpus={} (saturating): reader p99 = {:.2} ms",
        n_cpus,
        p99_saturating as f64 / 1e6,
    );
    let p99_isolated = mixed_load_p99_ns((n_cpus / 2).max(1), n_queries);
    eprintln!(
        "  writer_pool=num_cpus/2={} (isolated):  reader p99 = {:.2} ms",
        (n_cpus / 2).max(1),
        p99_isolated as f64 / 1e6,
    );
    let ratio = p99_saturating as f64 / p99_isolated.max(1) as f64;
    eprintln!(
        "  saturating / isolated ratio = {ratio:.2}x  (>1 means the dual-pool isolation helps)"
    );

    // Tiny sleep to let stderr flush before criterion's summary.
    thread::sleep(Duration::from_millis(50));
}

criterion_group!(benches, bench_mixed_load);
