# Scale benches

Long-running runners that need the release profile. Two flavors live
together here:

- **Stress runners** that exercise the writer + manifest stack at
  extreme size (`supertable_100gb_laptop`, `supertable_1m_segments`).
- **Pinned-recall assertion runners** (`fts_recall`, `vector_recall`)
  that build a non-trivial corpus, run search against a Tantivy /
  brute-force oracle, and panic if recall drops below the pinned
  threshold. These exist as benches rather than tests because the
  20K-doc Tantivy build + 50-query brute-force ground truth takes
  minutes in debug but seconds in release; `cargo bench` runs them
  in release by default.

Each runner has its own env-var knobs and prints single-line
summaries per phase on stderr. The `scale` bundle dispatches via a
positional arg after `--`:

```sh
cargo bench --features bench-diagnostics --bench scale
cargo bench --features bench-diagnostics --bench scale -- 100gb_laptop
cargo bench --features bench-diagnostics --bench scale -- 1m_segments
cargo bench --features bench-diagnostics --bench scale -- fts_recall
cargo bench --features bench-diagnostics --bench scale -- vector_recall
```

## Sequence

`fts_recall` + `vector_recall` first (correctness gates — cheap, run
in seconds), then the size-stress runners (minutes to ~35 min
depending on knob).

### `fts_recall` — pinned FTS recall at 20K docs

`benches/scale/fts_recall.rs`. WAND + BMW must return the same
top-k set as Tantivy on 20 overlap-heavy query shapes (adjacent
Zipfian-rank terms, dominator-plus-mid-tier, prefix expansion,
etc.). Catches BMW UB-undercount regressions that the 60-doc
unit-test scale can't expose because the heap threshold never
gets tight enough there. Builds the corpus + Tantivy index once
via `OnceLock`; release-mode runtime is ~5 s.

### `vector_recall` — pinned vector recall at 10K × 384

`benches/scale/vector_recall.rs`. 4 pinned-threshold checks
against brute-force ground truth on a planted-cluster Gaussian
corpus. Asserts recall@10 ≥ 0.90/0.95 at default options
(`nprobe=8/32`, `rerank_mult=20`) and that recall increases
monotonically with `nprobe` and `rerank_mult`. Release-mode
runtime is ~2 s; debug mode is ~3–4 min because k-means +
brute-force scans dominate.

### `supertable_100gb_laptop` — laptop-scale build through LocalFS

Builds a supertable via the LocalFS-backed `StorageProvider` at
the scale where on-disk index size lands around 100 GB, then
reopens via a fresh handle and asserts recovered state matches.

| Scale knob | Doc count | On-disk size | Notes |
|---|---:|---:|---|
| Default | 10M | ~23 GB | Floor for "realistic"; runs in minutes |
| `INFINO_BENCH_100GB=1` | 43M | ~100 GB | Plan's target scale; ~35 min wall, ~110 GB free SSD |
| `INFINO_BENCH_M14_N_DOCS=1000000` | 1M | ~2.3 GB | Smoke |

Phases:

1. **Build** — stream N docs through `writer.append + writer.commit` in
   1M-doc chunks via LocalFS.
2. **Open-and-verify** — drop the producer, reopen via
   `Supertable::open`, assert `manifest_id`, `n_segments`,
   `n_docs_total` recover.
3. **Cold-query** — open a fresh consumer with a disk cache
   attached; first query triggers parallel range-fetch + pwrite
   + mmap through `DiskCacheStore`; reports cold-pass wall +
   `n_cold_fetches` + cache bytes.
4. **Warm-query** — repeats the query; asserts
   `n_cold_fetches_delta == 0` and reports the cold/warm
   speedup ratio.

Smoke run (100K docs, M4 Max) confirms ≈ **2.25 KB/doc** on disk,
matching the 2.3 KB/doc projection that drives the 43M-doc →
100 GB sizing.

### `supertable_1m_segments` — manifest stress at petabyte scale

Synthetic manifest stress via direct manifest manipulation —
exercises the manifest open / refresh / list-load path at segment
counts that exceed any real workload by orders of magnitude.

| Scale knob | Configuration | Total segments |
|---|---|---:|
| Default | 10 parts × 10 segments | 100 (smoke) |
| `INFINO_BENCH_M15D_MEDIUM=1` | 100 × 100 | 10 K |
| `INFINO_BENCH_M15D_FULL=1` | 100 × 10 K | **1 M** |

Reports manifest build wall time, open wall time, list-load wall
time, then a sibling-commit refresh check (advancement must be
detected on a refresh after a sibling-side commit). 1M-segment
runs validate single-node feasibility at the manifest layer — at
~1 KB/segment-entry the manifest is ~1 GB resident, well under the
budget of any realistic deployment.
