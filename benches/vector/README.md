# Vector benches

All vector benches live in one bundled criterion binary (`cargo
bench --bench vector`). Filter with a criterion regex, e.g.
`cargo bench --bench vector -- search_vs_lance`.

Default scale is 1M docs × 384-dim cosine on planted-cluster
Gaussian corpora. `INFINO_BENCH_FULL=1` bumps to 10M docs ×
384-dim. Reference numbers from an Apple M4 Max baseline.

LanceDB is pinned at `=0.27.2` (0.x API churns across versions).

## Sequence

Build → search → recall calibration → Lance comparisons →
diagnostic profiler → supertable-layer.

### `superfile/build` — single-segment vector build throughput

K-means-dominated (5 iterations of Lloyd's IVF training). Build
throughput at 1M × 384 lands around **50 K docs/s** single
thread (~20 s wall). Default-rayon-pool gives an 11.7× speedup;
the recent k-means cleanups (final-iter assignments returned
alongside centroids; per-thread accumulators with pairwise
reduction for the update step) shaved ~3 s off the ~15 s
`finish()` call.

### `superfile/search` — single-segment vector kNN latency

`top-10`, cosine, default options:

| Configuration | 1M docs | 10M docs |
|---|---:|---:|
| Default (`nprobe=8`, `rerank_mult=20`) | 5.35 ms | **6.71 ms** |
| `nprobe=128`, `rerank_mult=20` | 9.33 ms | 19.15 ms |
| `nprobe=8`, `rerank_mult=100` | 5.81 ms | 8.55 ms |

Even at the most aggressive recall setting the 10M number is
inside the 20 ms target. Wall time at the default is dominated by
the f32 rerank stage (each rerank reads a 384 × 4-byte vector);
SIMD `f32x8` distance kernel via `bytemuck::try_cast_slice`
(zero-copy when 4-aligned, which it always is in practice).

### `superfile/build_vs_lance` — build + footprint + cold-load

Single-threaded build, criterion median over 10 samples.

| Metric | infino | LanceDB | Margin |
|---|---:|---:|---:|
| Build time, 1 thread | **20.2 s** | 23.8 s | infino **1.18× faster** |
| Throughput | **50 K docs/s** | 42 K docs/s | infino **1.18× faster** |
| Index size on disk | 1515.94 MiB | 1523.96 MiB | tied |
| Cold-load (default, CRC on) | 132.6 ms | **0.26 ms** | Lance ~500× faster |
| Cold-load (`verify_crc=false`) | **1.05 ms** | 0.26 ms | Lance ~4× faster |
| First-query latency (after cold open) | **5.86 ms** | 15.90 ms | infino **2.7× faster** |

Index size parity is a function of the f32 rerank vectors (used
by both engines). The cold-load gap closes 130× when the caller
opts in to skip CRC verification.

### `superfile/search_vs_lance` — head-to-head kNN at calibrated recall

Calibration picks each engine's lowest-p50 `(probe, refine)`
config that hits a recall@10 target — neither engine gets to
pick a number where the other is at a disadvantage.

| Recall@10 ≥ | infino (calibrated) | Lance (calibrated) | Margin |
|---|---:|---:|---:|
| **0.90** | **4.97 ms** (`p=1, r=256`) | 11.19 ms (`p=5, r=256`) | **2.25× faster** |
| **0.95** | **5.08 ms** (`p=5, r=256`) | 11.18 ms (`p=10, r=256`) | **2.20× faster** |
| **0.99** | **5.07 ms** (`p=5, r=1024`) | 11.30 ms (`p=10, r=256`) | **2.23× faster** |

Both engines hit recall=1.00 at the 0.95+ tiers — the
interesting fact is the *cost* of getting there (~5.1 ms vs
~11.2 ms). Recall is enforced by
`tests/superfile/vector/against_lance.rs`.

### `superfile/open_profile` — diagnostic profiler

Profiles `VectorReader::open` at 1M scale; prints the per-phase
breakdown emitted by the `INFINO_PROFILE_OPEN=1`
instrumentation in `src/superfile/vector/reader.rs`. One-off
diagnostic — not a published headline number.

### `supertable/search_vs_lance` — multi-segment vector head-to-head

Same Gaussian planted-cluster corpus, supertable sharded into 4
segments with `n_cent_per_segment = 16` (total 64 IVF clusters,
matching Lance's `num_partitions`).

| Recall target | Supertable p50 | Lance p50 | Ratio |
|---|---:|---:|---:|
| ≥ 0.90 | 11.08 ms (probe=4, refine=256) | 16.00 ms (probe=1, refine=256) | **1.44×** |
| ≥ 0.95 | 11.32 ms (probe=16, refine=64) | 28.42 ms (probe=100, refine=256) | **2.51×** |
| ≥ 0.99 | 12.22 ms (probe=16, refine=256) | 22.03 ms (probe=5, refine=256) | **1.80×** |

Cross-engine correctness enforced by `tests/supertable/query/against_lance.rs`
— at each recall target, brute-force-relative recall ≥ target on
both engines, and cross-engine Jaccard ≥ {0.75, 0.85, 0.95} for
{0.90, 0.95, 0.99}. Measured Jaccards over the 500-query batch:
0.78 / 0.89 / 0.99.
