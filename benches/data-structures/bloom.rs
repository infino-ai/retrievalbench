//! Comparator bench: supertable's bloom vs `fastbloom`.
//!
//! Measures `contains()` throughput across the two probe shapes
//! the bloom sees in real skip-pruning workloads:
//!
//! 1. **Confirm-present** — the probed key was inserted into the
//!    bloom (every K-bit position is set). The bloom correctly
//!    returns `true`. This path drives the "yes, scan this
//!    segment" branch of skip pruning.
//! 2. **Confirm-absent** — the probed key was *not* inserted (some
//!    K-bit position is unset). The bloom correctly returns
//!    `false`. This path drives the "no, skip this segment"
//!    branch. **In practice this dominates** — most query terms
//!    miss in most superfiles, which is the whole point of having
//!    a skip-prune layer.
//!
//! Both rows are throughput — higher M-element/s is better in
//! both cases. The naming "miss" got reused for "the bloom
//! correctly says no, the key isn't here" rather than the cache-
//! miss / branch-miss sense; we use "confirm-absent" to keep that
//! unambiguous.
//!
//! Sizes: 8 KiB, 32 KiB, 64 KiB. We populate to ~100 items per
//! block (~7% target FPR for the chosen sizing).
//!
//! ## Reference numbers (M4 Max, M-element/s)
//!
//! | Probe shape       | Size   | Ours | fastbloom | Ratio |
//! |-------------------|--------|-----:|----------:|------:|
//! | Confirm-present   | 8 KiB  |  180 |       121 | 1.49× |
//! | Confirm-present   | 32 KiB |  177 |       121 | 1.46× |
//! | Confirm-present   | 64 KiB |  179 |       120 | 1.49× |
//! | Confirm-absent    | 8 KiB  |  182 |        87 | 2.09× |
//! | Confirm-absent    | 32 KiB |  178 |        91 | 1.96× |
//! | Confirm-absent    | 64 KiB |  180 |        91 | 1.98× |
//!
//! Two design decisions land the win:
//!
//! - **XXH3-64 hash backbone** (was SipHash-1-3). XXH3 is ~3×
//!   faster than SipHash-1-3 on small inputs, and skip-prune
//!   workloads spend most of their time on the confirm-absent
//!   path where hash cost dominates the inner loop. We don't
//!   need HashDoS resistance — terms come from a closed corpus.
//! - **Portable SIMD via `wide::u64x4`**: `contains` builds a
//!   K-bit test mask once, then checks `(block & mask) == mask`
//!   in two SIMD AND-NOT-OR-reduce operations covering all 8
//!   block words. Lowers to AVX2 on x86_64-with-avx2, NEON on
//!   aarch64. Side effect: the SIMD path doesn't short-circuit
//!   on the first absent bit — confirm-present and confirm-absent
//!   paths both do the same fixed work, giving uniform
//!   throughput (~180 M-elem/s either way).
//!
//! Hot/cold-cache split isn't modeled — at our sizes the whole
//! bloom fits in L1 on M-series Mac and modern x86. A future
//! revision could add a cache-flush ritual between iterations to
//! see the cold-path tax.

#![deny(clippy::unwrap_used)]

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group};
use fastbloom::BloomFilter;

use infino::supertable::manifest::bloom::{BLOCK_BYTES, Bloom, BloomBuilder};

const SIZES_BYTES: &[usize] = &[8 * 1024, 32 * 1024, 64 * 1024];
const N_PROBES: usize = 10_000;

/// Build our supertable bloom + populate to the target load.
fn build_ours(n_blocks: usize, n_items: usize) -> Bloom {
    let mut b = BloomBuilder::with_n_blocks(n_blocks);
    for i in 0..n_items {
        b.insert(format!("term{i}").as_bytes());
    }
    b.finish()
}

/// Build fastbloom + populate to the target load.
fn build_fastbloom(n_bits: usize, n_items: usize) -> BloomFilter {
    let mut bf = BloomFilter::with_num_bits(n_bits).expected_items(n_items);
    for i in 0..n_items {
        bf.insert(format!("term{i}").as_bytes());
    }
    bf
}

fn bench_contains_confirm_present(c: &mut Criterion) {
    let mut g = c.benchmark_group("bloom_contains_confirm_present");
    g.throughput(Throughput::Elements(N_PROBES as u64));
    for &size_bytes in SIZES_BYTES {
        let n_blocks = size_bytes / BLOCK_BYTES;
        let n_items = n_blocks * 100;
        let label = format!("{}KiB", size_bytes / 1024);

        // ours
        {
            let bloom = build_ours(n_blocks, n_items);
            let probes: Vec<Vec<u8>> = (0..N_PROBES)
                .map(|i| format!("term{}", i % n_items).into_bytes())
                .collect();
            g.bench_function(format!("ours_{label}"), |b| {
                b.iter(|| {
                    let mut hits = 0usize;
                    for p in &probes {
                        if bloom.contains(black_box(p)) {
                            hits += 1;
                        }
                    }
                    black_box(hits)
                });
            });
        }

        // fastbloom
        {
            let bf = build_fastbloom(size_bytes * 8, n_items);
            let probes: Vec<Vec<u8>> = (0..N_PROBES)
                .map(|i| format!("term{}", i % n_items).into_bytes())
                .collect();
            g.bench_function(format!("fastbloom_{label}"), |b| {
                b.iter(|| {
                    let mut hits = 0usize;
                    for p in &probes {
                        if bf.contains(black_box(p)) {
                            hits += 1;
                        }
                    }
                    black_box(hits)
                });
            });
        }
    }
    g.finish();
}

fn bench_contains_confirm_absent(c: &mut Criterion) {
    let mut g = c.benchmark_group("bloom_contains_confirm_absent");
    g.throughput(Throughput::Elements(N_PROBES as u64));
    for &size_bytes in SIZES_BYTES {
        let n_blocks = size_bytes / BLOCK_BYTES;
        let n_items = n_blocks * 100;
        let label = format!("{}KiB", size_bytes / 1024);

        // ours — probes are not inserted; bloom should answer
        // false (a few false-positives at FPR rate, but the
        // dominant path is confirm-absent).
        {
            let bloom = build_ours(n_blocks, n_items);
            let probes: Vec<Vec<u8>> = (0..N_PROBES)
                .map(|i| format!("absent{i}").into_bytes())
                .collect();
            g.bench_function(format!("ours_{label}"), |b| {
                b.iter(|| {
                    let mut hits = 0usize;
                    for p in &probes {
                        if bloom.contains(black_box(p)) {
                            hits += 1;
                        }
                    }
                    black_box(hits)
                });
            });
        }

        // fastbloom
        {
            let bf = build_fastbloom(size_bytes * 8, n_items);
            let probes: Vec<Vec<u8>> = (0..N_PROBES)
                .map(|i| format!("absent{i}").into_bytes())
                .collect();
            g.bench_function(format!("fastbloom_{label}"), |b| {
                b.iter(|| {
                    let mut hits = 0usize;
                    for p in &probes {
                        if bf.contains(black_box(p)) {
                            hits += 1;
                        }
                    }
                    black_box(hits)
                });
            });
        }
    }
    g.finish();
}

criterion_group!(
    benches,
    bench_contains_confirm_present,
    bench_contains_confirm_absent,
);
