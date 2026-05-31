# FTS benches

Superfile FTS benches live in `cargo bench --bench superfile_fts`;
supertable FTS comparisons live in `cargo bench --bench
supertable_all`. Filter to a subset with a criterion regex on the
group or bench name, e.g. `cargo bench --bench superfile_fts --
search_vs_tantivy`.

Default scale is 1M docs (Zipfian, 200 tokens/doc, 10K vocabulary).
`INFINO_BENCH_FULL=1` bumps to 10M docs. Reference numbers are
from an Apple M4 Max baseline; absolute numbers vary by machine
but ratios stay stable.

## Sequence

The benches are ordered build → search → microbench, matching how a typical perf investigation
flows: prove the build path lands the right bytes, prove the
search path queries them fast, then isolate the codec.

### `superfile/build` — single-segment ingestion throughput

| Configuration | Time | Throughput |
|---|---:|---:|
| 1 thread | 9.91 s | 101 K docs/s |
| rayon (default threads) | 907 ms | **1.10 M docs/s** |

Posting accumulation uses `Vec<(u32, u32)>` per term + mimalloc;
the rayon shard splits the input doc range across worker cores
and each shard emits a self-contained FTS blob (composes
naturally with the multi-segment commit shape).

### `superfile/build_vs_tantivy` — head-to-head ingestion

| Configuration | Time | Throughput | vs infino |
|---|---:|---:|---:|
| infino (1 thread) | 9.75 s | 103 K docs/s | — |
| Tantivy 0.26.1 (1 indexing thread) | 15.60 s | 64 K docs/s | **infino 1.6× faster** |
| infino (rayon, default threads) | 907 ms | 1.10 M docs/s | — |
| Tantivy 0.26.1 (default — 4–8 threads) | 2.22 s | 451 K docs/s | **infino 2.4× faster** |

Same 1M-doc Zipfian corpus, both engines emit one in-memory FTS
blob per iteration.

### `superfile/search` — single-segment BM25 latency

Single-engine sweep over WAND vs BMM dispatchers. Default
(`bm25_common_term_top10`) at 1M docs is **5.43 µs** via the
`SuperfileReader` wrapper; the direct `FtsReader::search` path is
~5 µs (the wrapper adds no measurable overhead). The per-shape
breakdown shows up in the head-to-head table below.

### `superfile/search_vs_tantivy` — head-to-head search latency

Both sides receive pre-tokenized input so the measurement
isolates BM25 retrieval (no parser/tokenizer cost).

| Query shape | infino | Tantivy 0.26.1 | Ratio |
|---|---:|---:|---:|
| `single_rare_top10` | **253 ns** | 17.7 µs | **70× faster** |
| `single_common_top10` | **5.85 µs** | 336 µs | **57× faster** |
| `two_term_or_top10` | **103 µs** | 1.37 ms | **13× faster** |
| `three_wide_top10` (rank 1 + 50 + 100) | **1.28 ms** | 4.52 ms | **3.5× faster** |
| `three_similar_top10` (rank 50 + 51 + 52) | **5.94 ms** | 8.42 ms | **1.4× faster** |
| `five_term_top10` (rank 50 – 54) | **10.0 ms** | 21.4 ms | **2.1× faster** |

Recall is enforced by `tests/superfile/fts/recall_at_scale.rs`
(20 stress tests at 20K docs, top-k against a Tantivy oracle) +
`tests/superfile/fts/against_tantivy.rs` (planted-truth oracle at
60 docs).

### `superfile/bm25_decode` — posting-decode microbench

Isolated PFOR-delta posting decode rate (the codec primitive that
the search hot path runs hundreds of thousands of times per
multi-term query). The win on `three_similar_top10` (24.2 ms →
5.94 ms in a single commit) came from splitting `TermCursor::skip_to`
into an `#[inline(always)]` within-block fast path + a `#[cold]`
cross-block helper — this microbench is the inner-loop probe that
made that win measurable.

### `supertable/search_vs_tantivy` — multi-segment search head-to-head

Same Zipfian 1M-doc corpus as `superfile/search_vs_tantivy`, both
engines sharded into 4 segments (Tantivy with `NoMergePolicy` so
its segment count stays at 4).

| Query shape | Supertable | Tantivy | Ratio |
|---|---:|---:|---:|
| `single_rare` (term in ~1 doc) | 35.2 µs | 94.9 µs | **2.69×** |
| `single_common` (long posting list) | 38.9 µs | 553.7 µs | **14.2×** |
| `two_term_or` | 78.6 µs | 2.01 ms | **25.6×** |
| `three_wide_or` | 405 µs | 4.76 ms | **11.8×** |
| `three_similar_or` | 1.59 ms | 8.68 ms | **5.45×** |
| `five_term_or` | 3.04 ms | 21.8 ms | **7.17×** |
| `prefix` (10-term expansion) | 7.11 ms | 41.5 ms | **5.83×** |

Rare-term + prefix wins come from the manifest's bloom +
term-range skip-prune at the segment-list layer (Tantivy 0.26
has no equivalent). Per-shape wins compose the
single-segment speedup × rayon fan-out across segments.

### `supertable/bloom_contains` — manifest skip-bloom microbench

The supertable's per-FTS-column term-presence bloom is the
load-bearing primitive for skip pruning — every query term ×
every segment is one `contains()` call before any payload byte is
touched. Compared against [`fastbloom`](https://crates.io/crates/fastbloom)
0.17 at three sizes; throughput in M-elements/s (higher is
better).

| Probe shape | Size | infino | `fastbloom` | Margin |
|---|---|---:|---:|---:|
| Confirm-present | 8 KiB | 180 | 121 | **1.49×** |
| Confirm-present | 32 KiB | 177 | 121 | **1.46×** |
| Confirm-present | 64 KiB | 179 | 120 | **1.49×** |
| Confirm-absent | 8 KiB | 182 | 87 | **2.09×** |
| Confirm-absent | 32 KiB | 178 | 91 | **1.96×** |
| Confirm-absent | 64 KiB | 180 | 91 | **1.98×** |

Confirm-absent (~99% of probes in real skip-prune workloads) wins
2.0× via XXH3-64 (3× faster than `fastbloom`'s default hash on
small inputs) + a portable SIMD bit-test (`wide::u64x4`,
AVX2/NEON) + a single-cache-line block layout (one 64 B block per
probe). Detailed design notes in [`../README.md` § "Head-to-head
vs `fastbloom`"](../README.md).
