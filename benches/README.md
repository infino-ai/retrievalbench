# infino benchmarks

[Criterion](https://github.com/bheisler/criterion.rs) benchmarks for
infino's superfile + supertable pipeline, with head-to-head
comparisons against Tantivy (FTS) and LanceDB (vector). Result
tables in this document are **auto-populated** from the criterion
output — re-run with `INFINO_BENCH_UPDATE_README=1` to refresh in
place.

## Reference machine

- Apple M4 Max (12 P-cores + 4 E-cores, 16 total)
- 64 GB unified memory
- macOS 15
- Single-machine numbers; absolute timings vary by hardware, but
  cross-engine **ratios** stay stable across machines.

## Methodology

### Corpus

Deterministic synthetic corpus shared by infino and the reference
engine (Tantivy / Lance) bit-for-bit. Generation lives in
[`utils/corpus.rs`](utils/corpus.rs).

**FTS text** — Zipfian term distribution over a 10K-term vocabulary
(`term00001`…`term09999`), 200 body tokens per doc plus a
doc-unique `doc{i:07}` prefix. The per-doc-unique prefix is the
**df=1 correctness probe**: it appears in exactly one doc by
construction, so cross-engine top-K agreement on it is a strict
oracle.

- Per-doc on-the-wire: `"doc0000000 term00050 term00100 …"`
- Per-doc size: `10 (doc prefix) + 200 × (1 space + 9 term chars) = 2010 bytes` (ASCII)
- Total raw corpus at scale: ~2.0 GB at 1M docs; ~20.1 GB at 10M docs

**Vector** — Gaussian planted-cluster corpus in `dim = 384`. Each
doc is a single 384-dim `f32` vector normalized to unit length
(cosine metric). Cluster centers come from `3·N(0, 1)` per dim;
per-doc Gaussian noise with norm `0.3·√DIM ≈ 5.9` (about 10% of the
cluster radius), so cluster identity is recoverable but boundaries
are non-trivial.

- Per-doc size: `384 × 4 bytes = 1536 bytes` (raw f32)
- Total raw corpus: ~1.5 GB at 1M docs; ~15.4 GB at 10M docs
- Cluster count: ~`sqrt(n_docs)` (matches LanceDB's
  `num_partitions` default; same IVF cluster shape on both engines)
- Calibration queries: 5%-perturbed copies of random doc vectors
  (so the brute-force top-1 is the doc itself; brute-force top-K
  is its nearest cluster mates — a realistic recall workload).

### Scales

| Bundle      | Default scale         | Raw corpus | Rationale |
|-------------|-----------------------|-----------:|-----------|
| `fts/superfile`     | 1M docs × 2010 B   | ~2.0 GB | Single-superfile shape — production superfiles are rarely much larger; the supertable handles scale-out. |
| `fts/supertable`    | 10M docs × 2010 B  | ~20.1 GB | Scale-out shape; sharded into N superfiles (writer-pool size dictates N). |
| `vector/superfile`  | 1M × dim=384 × f32 | ~1.5 GB | Single-segment vector index — IVF + RaBitQ + rerank measurement. |
| `vector/supertable` | 10M × dim=384 × f32 | ~15.4 GB | Cross-segment kNN fan-out + global top-K merge. |
| `scale`     | 10K–43M docs (configurable) | varies | Calibrated recall + memory budget + 100 GB-on-disk stress. |

The 64 GB reference machine fits the 10M-doc supertable corpora in
RAM with headroom for engine indices + ground-truth caches; smaller
boxes need either swap or `INFINO_BENCH_FULL`-style sub-scale
overrides.

### Apples-to-apples — FTS (infino vs Tantivy 0.26.1)

| Setting | infino | Tantivy |
|---------|--------|---------|
| Tokenizer | `AsciiLowerTokenizer` (ASCII split + lowercase) | `SimpleTokenizer` + `LowerCaser` (identical token stream on alnum-only corpus) |
| Posting record type | `(doc_id, tf)` | `IndexRecordOption::WithFreqs` (no positions) |
| BM25 params | k1=1.2, b=0.75 (Lucene defaults) | same |
| Merge policy | n/a (one superfile per writer-pool thread per commit) | `NoMergePolicy` (no merges → comparable segment count) |
| Writer threads (default) | `cpus/2` (= 8 on a 16-core machine) — leaves room for queries | heap-derived (~ 8 at 2 GB heap) |
| Heap budget | n/a (memory-bounded by writer pool size) | 2 GB (supertable bench) / 500 MB (superfile bench) |

The writer-thread budget for infino can be overridden via
`INFINO_SUPERTABLE__WRITER_THREADS=N` to match Tantivy's effective
output segment count for stricter apples-to-apples cardinality
comparisons.

### Apples-to-apples — vector (infino vs LanceDB)

| Setting | infino | Lance |
|---------|--------|-------|
| Index family | IVF + 1-bit RaBitQ + f32 rerank | IVF + 8-bit PQ + f32 refine |
| Total cluster count | `n_cent` (single-superfile) or `n_cent / N_SEGMENTS` per superfile × N (supertable) | `num_partitions = n_cent` |
| Quantization footprint | 1-bit RaBitQ → ~48 B / vector at dim=384 | 8-bit PQ with `num_sub_vectors = dim / 6 = 64` → 64 B / vector |
| Metric | cosine (normalized input) | cosine |
| Calibration grid | symmetric `(probe, refine)` across both engines (`probes ∈ {1, 5, 10, 25, 50, 100, 200, 400, 800}`, `refines ∈ {1, 4, 16, 64, 256, 1024}`) | same grid |
| Ground truth | brute-force exact kNN on the same corpus | same |
| Query API | `reader.search(&[f32], …)` — takes a borrowed slice | `Vec<f32>` → `block_on` → DataFusion plan → `RecordBatch` stream → downcast (≈ 70–260 µs of per-call overhead infino doesn't pay) |
| Threads at build | rayon-default (uncapped) | Lance's default Tokio + rayon pool |

Lance's per-query plan/runtime overhead is **included** in the
head-to-head numbers — it's what a real Lance caller pays via the
public API. Not subtracted.

### Correctness gates

Every bench run executes its correctness phase **before** timing,
unconditionally (criterion's filter affects only the timing
iterations, not the bench function's setup). The phases:

FTS:
- **BMW vs brute-force oracle** (infino-only). Calls `search(..., k=∞)`
  to disable BMW's threshold-driven pruning, then asserts the
  position-by-position scores match BMW's top-10 within
  `epsilon = 1e-4`. Strong check — catches BMW skip bugs, BMM
  partition bugs, and posting-decode bugs that affect ranking
  without needing Tantivy as oracle.
- **df=1 cross-engine match against Tantivy**. The corpus's
  per-doc unique `doc{i:07}` token has df=1, so both engines must
  return the same single doc.
- Strict top-K cross-engine agreement is **not** asserted at the
  bench scale because Zipfian corpora produce deeply tied score
  pools at the top-K boundary, and both engines tiebreak
  arbitrarily within those pools. Strict oracle correctness lives
  at smaller scale in
  [`tests/superfile/fts/against_tantivy.rs`](../tests/superfile/fts/against_tantivy.rs)
  (60-doc planted-truth) and
  [`benches/scale/fts_recall.rs`](scale/fts_recall.rs) (20K-doc
  Zipfian strict top-K).

Vector:
- **Both engines hit recall@10 ≥ 0.80** at a high-recall correctness
  config (`nprobe=64, rerank_mult=256` for infino; `probes=64,
  refine=256` for Lance) against the brute-force kNN ground truth on
  a 20-query battery. Catches catastrophic pipeline regressions
  (broken clustering, quantization bug, rerank shortlist sizing).
- **Cross-engine Jaccard parity** at calibrated `(probe, refine)`
  points is asserted at smaller scale in
  [`benches/scale/oracle_calibrated_recall_targets_match_lance.rs`](scale/oracle_calibrated_recall_targets_match_lance.rs)
  (5K × 384, 500 queries, 3 recall targets). The supertable bench
  here only asserts each engine clears the recall floor
  independently; the calibration sweep then finds each engine's
  lowest-p50 config for the timing comparison.

### Result anchors

The result sections below are wrapped in
`<!-- BEGIN: bench/... --> <!-- END: bench/... -->` markers; the
bench's markdown emitter rewrites the content between these markers
when `INFINO_BENCH_UPDATE_README=1` is set. Re-running a bench with
a criterion filter only refreshes the matching section.

## Layout

Single criterion binary per topic. `Cargo.toml` has four
`[[bench]]` stanzas: `fts`, `vector`, `e2e`, `scale`.

```
benches/
├── README.md         # this file
├── fts/
│   ├── README.md     # detailed per-shape commentary
│   ├── main.rs       # criterion_main!(superfile, supertable)
│   ├── superfile.rs  # ingest + search (1M docs) — single file
│   └── supertable.rs # ingest + search (10M docs)
├── vector/
│   ├── README.md
│   ├── main.rs       # criterion_main!(superfile, supertable)
│   ├── superfile.rs  # ingest + search (1M × 384) — single file
│   └── supertable.rs # ingest + search (10M × 384)
├── e2e/
├── scale/
│   ├── fts_recall.rs
│   ├── vector_recall.rs
│   ├── supertable_100gb_laptop.rs
│   ├── supertable_1m_segments.rs
│   ├── supertable_ingest_once.rs
│   └── oracle_calibrated_recall_targets_match_lance.rs
└── utils/
    ├── corpus.rs    # deterministic Zipfian + vector corpora
    ├── lance.rs     # LanceDB harness for vector head-to-head
    └── markdown.rs  # bench-result emitter + README rewriter
```

## Invocation

```sh
# Run everything (cold ~30 min for fts on 64 GB / M4 Max)
cargo bench --bench fts
cargo bench --bench vector
cargo bench --bench e2e
cargo bench --bench scale

# Filter to a sub-group (criterion regex/prefix on the group name)
cargo bench --bench fts -- superfile_fts_build           # superfile FTS ingest
cargo bench --bench fts -- superfile_fts_search          # superfile FTS search
cargo bench --bench fts -- supertable_fts_build          # supertable FTS ingest
cargo bench --bench fts -- supertable_fts_search         # supertable FTS search
cargo bench --bench vector -- superfile_vec_build        # superfile vector ingest
cargo bench --bench vector -- superfile_vec_search       # superfile vector search
cargo bench --bench vector -- supertable_vec_build       # supertable vector ingest
cargo bench --bench vector -- supertable_vec_search      # supertable vector search
cargo bench --bench fts -- _build                        # both FTS ingest groups
cargo bench --bench vector -- _search                    # both vector search groups

# Knobs (env-driven, via the standard config stack)
INFINO_SUPERTABLE__WRITER_THREADS=32 cargo bench --bench fts -- supertable_fts_build
INFINO_BENCH_UPDATE_README=1 cargo bench --bench fts        # rewrites the FTS result sections
INFINO_BENCH_UPDATE_README=1 cargo bench --bench vector     # rewrites the vector result sections

# Scale bench (release-only correctness oracles + stress runners)
cargo bench --bench scale -- supertable_ingest_once         # single-shot 10M FTS ingest head-to-head
cargo bench --bench scale -- oracle_calibrated_recall       # supertable-vs-Lance recall + Jaccard
cargo bench --bench scale -- fts_recall                     # 20K-doc Zipfian strict FTS recall
cargo bench --bench scale -- vector_recall                  # vector pinned-recall thresholds
```

The correctness phase runs unconditionally on every invocation;
filtering to a search group still validates the BMW oracle + df=1
cross-engine match before timing starts.

---

## Results

### FTS — superfile (single-segment, 1M docs)

<!-- BEGIN: bench/fts/superfile/ingest -->
### Superfile FTS — ingest (1000000 docs, Zipfian, 200 tokens/doc, 10K vocab)

| Engine | Time | Throughput | Peak RSS | Median RSS | P90 RSS | Peak RSS Δ | vs Tantivy |
|--------|------|------------|----------|------------|---------|------------|------------|
| infino_1thread | 10.92 s | 91.5 K/s | 5.43 GiB | 3.93 GiB | 4.21 GiB | -0.8% no change | **infino wins, 1.9× faster than tantivy** |
| tantivy_1thread | 20.24 s | 49.4 K/s | 17.49 GiB | 13.43 GiB | 16.70 GiB | — | — |
| infino_rayon_default_threads | 2.08 s | 480.7 K/s | 5.46 GiB | 5.11 GiB | 5.43 GiB | -0.0% no change | **infino wins, 2.0× faster than tantivy** |
| tantivy_default_threads | 4.12 s | 242.5 K/s | 17.49 GiB | 13.43 GiB | 16.70 GiB | — | — |
| lance_fts | 15.15 s | 66.0 K/s | 17.49 GiB | 13.43 GiB | 16.70 GiB | — | **lance_fts wins, 1.3× faster than tantivy** |

<!-- END: bench/fts/superfile/ingest -->

<!-- BEGIN: bench/fts/superfile/search -->
### Superfile FTS — search (1000000 docs)

**OR queries:**

| Query             | infino     | infino RSS | Tantivy    | Tantivy RSS | CoreDB    | CoreDB RSS  | Lance FTS  | Lance RSS  | Winner                |
|-------------------|------------|------------|------------|-------------|------------|------------|------------|------------|-----------------------|
| single_rare       | 467 ns     | 3.92 GiB   | 7.44 µs    | 13.26 GiB   | 646 ns     | —          | 676.53 µs  | 13.26 GiB  | **infino wins, 15.9× faster than tantivy** |
| single_df1        | 217 ns     | 3.92 GiB   | 1.29 µs    | 13.26 GiB   | 263 ns     | —          | 455.57 µs  | 13.26 GiB  | **infino wins, 6.0× faster than tantivy** |
| single_common     | 8.97 µs    | 3.92 GiB   | 101.94 µs  | 13.26 GiB   | 84.39 µs   | —          | 63.43 ms   | 13.26 GiB  | **infino wins, 11.4× faster than tantivy** |
| two_term_or       | 151.42 µs  | 3.92 GiB   | 685.24 µs  | 13.26 GiB   | 117.51 µs  | —          | 22.19 ms   | 13.26 GiB  | **infino wins, 4.5× faster than tantivy** |
| three_wide_or     | 2.21 ms    | 3.92 GiB   | 4.80 ms    | 13.26 GiB   | 139.69 µs  | —          | 13.65 ms   | 13.26 GiB  | **infino wins, 2.2× faster than tantivy** |
| three_similar_or  | 8.99 ms    | 3.92 GiB   | 12.81 ms   | 13.26 GiB   | 104.06 µs  | —          | 30.30 ms   | 13.26 GiB  | **infino wins, 1.4× faster than tantivy** |
| five_term_or      | 17.84 ms   | 3.92 GiB   | 27.03 ms   | 13.26 GiB   | 181.78 µs  | —          | 62.85 ms   | 13.26 GiB  | **infino wins, 1.5× faster than tantivy** |

**AND queries:**

| Query             | infino     | infino RSS | Tantivy    | Tantivy RSS | CoreDB    | CoreDB RSS  | Lance FTS  | Lance RSS  | Winner                |
|-------------------|------------|------------|------------|-------------|------------|------------|------------|------------|-----------------------|
| two_term_and      | 174.52 µs  | 3.92 GiB   | 8.79 ms    | 13.26 GiB   | 118.31 µs  | —          | 1.28 ms    | 13.26 GiB  | **infino wins, 50.4× faster than tantivy** |
| three_wide_and    | 3.35 ms    | 3.92 GiB   | 5.23 ms    | 13.26 GiB   | 144.49 µs  | —          | 9.50 ms    | 13.26 GiB  | **infino wins, 1.6× faster than tantivy** |
| three_similar_and | 5.36 ms    | 3.92 GiB   | 7.03 ms    | 13.26 GiB   | 102.81 µs  | —          | 14.82 ms   | 13.26 GiB  | **infino wins, 1.3× faster than tantivy** |
| five_term_and     | 6.51 ms    | 3.92 GiB   | 7.13 ms    | 13.26 GiB   | 183.72 µs  | —          | 13.82 ms   | 13.26 GiB  | **infino wins, 1.1× faster than tantivy** |


**Per-algorithm probes** (infino-only, WAND+BMW vs MaxScore+BMM):

| Shape | WAND+BMW p50 | WAND+BMW Peak RSS | WAND+BMW Median RSS | WAND+BMW P90 RSS | WAND+BMW Peak RSS Δ | MaxScore+BMM p50 | MaxScore+BMM Peak RSS | MaxScore+BMM Median RSS | MaxScore+BMM P90 RSS | MaxScore+BMM Peak RSS Δ | Winner |
|-------|--------------|-------------------|---------------------|------------------|---------------------|------------------|-----------------------|-------------------------|----------------------|-------------------------|--------|
| wide_3_or | 7.21 ms | 3.92 GiB | 3.85 GiB | 3.86 GiB | -16.5% improved | 2.19 ms | 3.92 GiB | 3.85 GiB | 3.86 GiB | -16.5% improved | **MaxScore+BMM wins, 3.3× faster than WAND+BMW** |
| similar_3_or | 13.19 ms | 3.92 GiB | 3.85 GiB | 3.86 GiB | -16.5% improved | 8.97 ms | 3.92 GiB | 3.85 GiB | 3.86 GiB | -16.5% improved | **MaxScore+BMM wins, 1.5× faster than WAND+BMW** |
| similar_5_or | 38.68 ms | 3.92 GiB | 3.85 GiB | 3.86 GiB | -16.5% improved | 17.65 ms | 3.92 GiB | 3.85 GiB | 3.86 GiB | -16.5% improved | **MaxScore+BMM wins, 2.2× faster than WAND+BMW** |

<!-- END: bench/fts/superfile/search -->

### FTS — supertable (multi-superfile, 10M docs)

<!-- BEGIN: bench/fts/supertable/ingest -->
### Supertable FTS — ingest (10000000 docs, Zipfian, 200 tokens/doc, 10K vocab)

| Engine | Time | Throughput | Peak RSS | Median RSS | P90 RSS | Peak RSS Δ | vs Tantivy |
|--------|------|------------|----------|------------|---------|------------|------------|
| infino_auto_writer_pool | 50.86 s | 196.6 K/s | 28.37 GiB | 24.27 GiB | 26.68 GiB | -11.5% improved | **tantivy wins, 1.4× faster than infino** |
| tantivy_default_threads | 35.18 s | 284.3 K/s | 51.36 GiB | 49.29 GiB | 50.34 GiB | — | — |

*Output cardinality: infino emits `min(writer_pool.threads, total_rows)` superfiles per commit (auto = cpus/2). Tantivy emits one segment per internal worker thread per commit (≈ 8 × N_chunks segments with NoMergePolicy). Override the infino writer-thread count with `INFINO_SUPERTABLE__WRITER_THREADS=N` to match Tantivy's effective output segment count.*

<!-- END: bench/fts/supertable/ingest -->

<!-- BEGIN: bench/fts/supertable/search -->
### Supertable FTS — search (10000000 docs)

| Query | infino p50 | infino Peak RSS | infino Median RSS | infino P90 RSS | infino Peak RSS Δ | Tantivy p50 | Tantivy Peak RSS | Tantivy Median RSS | Tantivy P90 RSS | Tantivy Peak RSS Δ | Winner |
|-------|------------|-----------------|-------------------|----------------|-------------------|-------------|------------------|--------------------|-----------------|--------------------|--------|
| single_rare | 72.89 µs | 25.93 GiB | 25.08 GiB | 25.09 GiB | -12.1% improved | 43.51 µs | 50.59 GiB | 47.58 GiB | 47.58 GiB | — | **tantivy wins, 1.7× faster than infino** |
| single_common | 87.82 µs | 25.93 GiB | 25.08 GiB | 25.09 GiB | -12.1% improved | 4.87 ms | 50.59 GiB | 47.58 GiB | 47.58 GiB | — | **infino wins, 55.5× faster than tantivy** |
| two_term_or | 493.93 µs | 25.93 GiB | 25.08 GiB | 25.09 GiB | -12.1% improved | 1.29 ms | 50.59 GiB | 47.58 GiB | 47.58 GiB | — | **infino wins, 2.6× faster than tantivy** |
| three_wide_or | 4.69 ms | 25.93 GiB | 25.08 GiB | 25.09 GiB | -12.1% improved | 9.96 ms | 50.59 GiB | 47.58 GiB | 47.58 GiB | — | **infino wins, 2.1× faster than tantivy** |
| three_similar_or | 15.82 ms | 25.93 GiB | 25.08 GiB | 25.09 GiB | -12.1% improved | 21.57 ms | 50.59 GiB | 47.58 GiB | 47.58 GiB | — | **infino wins, 1.4× faster than tantivy** |
| five_term_or | 35.55 ms | 25.93 GiB | 25.08 GiB | 25.09 GiB | -12.1% improved | 49.80 ms | 50.59 GiB | 47.58 GiB | 47.58 GiB | — | **infino wins, 1.4× faster than tantivy** |
| prefix | 81.01 ms | 25.93 GiB | 25.08 GiB | 25.09 GiB | -12.1% improved | 102.10 ms | 50.59 GiB | 47.58 GiB | 47.58 GiB | — | **infino wins, 1.3× faster than tantivy** |

<!-- END: bench/fts/supertable/search -->

### Vector — superfile (single-segment, 1M × dim=384)

<!-- BEGIN: bench/vector/superfile/ingest -->
### Superfile vector — ingest (1000000 docs × dim=384, Gaussian planted clusters, cosine)

| Engine | Time | Throughput | Peak RSS | Median RSS | P90 RSS | Peak RSS Δ | vs LanceDB |
|--------|------|------------|----------|------------|---------|------------|------------|
| infino | 22.23 s | 45.0 K/s | 4.09 GiB | 2.75 GiB | 3.72 GiB | — | **infino wins, 3.7× faster than lance** |
| lance | 81.88 s | 12.2 K/s | 5.60 GiB | 2.66 GiB | 3.12 GiB | — | — |

<!-- END: bench/vector/superfile/ingest -->

<!-- BEGIN: bench/vector/superfile/search -->
### Superfile vector — search (1000000 docs × dim=384, calibrated at recall targets)

| Recall target | infino p50 | infino Peak RSS | infino Median RSS | infino P90 RSS | infino Peak RSS Δ | Lance (probe, refine) | Lance p50 | Lance Peak RSS | Lance Median RSS | Lance P90 RSS | Lance Peak RSS Δ | Winner |
|---------------|------------|-----------------|-------------------|----------------|-------------------|-----------------------|-----------|----------------|------------------|---------------|------------------|--------|
| 0.90 | 213.60 µs | 3.74 GiB | 3.30 GiB | 3.31 GiB | — | (p=5, r=256) | 12.97 ms | 3.61 GiB | 3.54 GiB | 3.55 GiB | — | **infino wins, 60.7× faster than lance** |
| 0.95 | 345.37 µs | 3.74 GiB | 3.30 GiB | 3.31 GiB | — | (p=5, r=256) | 12.94 ms | 3.61 GiB | 3.54 GiB | 3.55 GiB | — | **infino wins, 37.5× faster than lance** |
| 0.99 | — | — | — | — | — | (p=5, r=256) | 12.89 ms | 3.61 GiB | 3.54 GiB | 3.55 GiB | — | — |

**infino default options** (`nprobe=8, rerank_mult=20` — user-facing latency baseline):

| Metric | Value |
|--------|-------|
| infino_default_options_top10 | 290.60 µs |
| infino_default_options_top10_peak_rss | 3.74 GiB |
| infino_default_options_top10_median_rss | 3.30 GiB |
| infino_default_options_top10_p90_rss | 3.31 GiB |
| infino_default_options_top10_peak_rss_delta | — |

<!-- END: bench/vector/superfile/search -->

### Vector — supertable (multi-superfile, 10M × dim=384)

<!-- BEGIN: bench/vector/supertable/ingest -->
### Supertable vector — ingest (10000000 docs × dim=384, sharded into 4 superfiles)

| Engine | Time | Throughput | Peak RSS | Median RSS | P90 RSS | Peak RSS Δ | vs LanceDB |
|--------|------|------------|----------|------------|---------|------------|------------|
| supertable | 252.80 s | 39.6 K/s | 26.73 GiB | 23.38 GiB | 25.21 GiB | — | **infino wins, 1.1× faster than lance** |
| lance | 283.21 s | 35.3 K/s | 37.46 GiB | 21.54 GiB | 37.12 GiB | — | — |

<!-- END: bench/vector/supertable/ingest -->

<!-- BEGIN: bench/vector/supertable/search -->
### Supertable vector — search (10000000 docs × dim=384, calibrated at recall targets)

| Recall target | supertable p50 | supertable Peak RSS | supertable Median RSS | supertable P90 RSS | supertable Peak RSS Δ | Lance (probe, refine) | Lance p50 | Lance Peak RSS | Lance Median RSS | Lance P90 RSS | Lance Peak RSS Δ | Winner |
|---------------|----------------|---------------------|-----------------------|--------------------|-----------------------|-----------------------|-----------|----------------|------------------|---------------|------------------|--------|
| 0.90 | 3.40 ms | 30.57 GiB | 30.56 GiB | 30.57 GiB | — | (p=10, r=256) | 81.85 ms | 29.27 GiB | 29.14 GiB | 29.17 GiB | — | **supertable wins, 24.0× faster than lance** |
| 0.95 | — | — | — | — | — | (p=10, r=256) | 76.81 ms | 29.27 GiB | 29.14 GiB | 29.17 GiB | — | — |
| 0.99 | — | — | — | — | — | (p=25, r=256) | 115.91 ms | 29.27 GiB | 29.14 GiB | 29.17 GiB | — | — |

<!-- END: bench/vector/supertable/search -->

## See also

- [`fts/README.md`](fts/README.md) — per-query-shape commentary on
  what each FTS bench is exercising and why.
- [`scale/README.md`](scale/README.md) — bench-scale lane:
  release-only correctness oracles + memory budget stress.
- [`tests/supertable/query/`](../tests/supertable/query/) —
  small-scale planted-truth oracles for both FTS and vector kNN
  (release-quick, run on every `cargo test`).
- [`docs/architecture/supertable.md`](../docs/architecture/supertable.md)
  — manifest data model, query fan-out, concurrency design.
