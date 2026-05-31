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
| `superfile_fts`     | 1M docs × 2010 B   | ~2.0 GB | Single-superfile shape — production superfiles are rarely much larger; the supertable handles scale-out. |
| `superfile_vector`  | 1M × dim=384 × f32 | ~1.5 GB | Single-segment vector index — IVF + RaBitQ + rerank measurement. |
| `supertable_all`    | 10M docs / 10M vectors | ~20.1 GB text + ~15.4 GB vectors | Scale-out shape; runs the supertable FTS and vector comparisons together. |
| `scale`     | 10K–43M docs (configurable) | varies | Opt-in calibrated recall + memory budget + 100 GB-on-disk stress. |

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

Single criterion binary per topic. `Cargo.toml` has `fts`, `vector`,
`hybrid`, and `scale` bench targets. Each superfile (1M) and supertable
(10M) search bench emits **hot / warm / cold** criterion groups; infino
warm/cold use object storage + disk cache, Lance warm/cold use `s3://`,
Tantivy warm/cold use a disk-backed index.

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
# Regular comparison benches
cargo bench --bench superfile_fts
cargo bench --bench superfile_vector
cargo bench --bench supertable_all

# Filter to a sub-group (criterion regex/prefix on the group name)
cargo bench --bench superfile_fts -- superfile_fts_build          # superfile FTS ingest
cargo bench --bench superfile_fts -- superfile_fts_search         # superfile FTS search
cargo bench --bench superfile_vector -- superfile_vec_build       # superfile vector ingest
cargo bench --bench superfile_vector -- superfile_vec_search      # superfile vector search
cargo bench --bench supertable_all -- supertable_fts_build        # supertable FTS ingest
cargo bench --bench supertable_all -- supertable_fts_search       # supertable FTS search
cargo bench --bench supertable_all -- supertable_all_build        # combined FTS + vector ingest
cargo bench --bench supertable_all -- supertable_vec_search       # supertable vector search

# Knobs (env-driven, via the standard config stack)
INFINO_SUPERTABLE__WRITER_THREADS=32 cargo bench --bench supertable_all -- supertable_fts_build
INFINO_BENCH_UPDATE_README=1 cargo bench --bench supertable_all

# Scale bench (release-only correctness oracles + stress runners)
cargo bench --features bench-diagnostics --bench scale -- supertable_ingest_once
cargo bench --features bench-diagnostics --bench scale -- oracle_calibrated_recall
cargo bench --features bench-diagnostics --bench scale -- fts_recall
cargo bench --features bench-diagnostics --bench scale -- vector_recall
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
| infino_1thread | 10.93 s | 91.5 K/s | 5.44 GiB | 3.92 GiB | 4.21 GiB | -0.5% no change | **infino wins, 1.8× faster than tantivy** |
| tantivy_1thread | 19.81 s | 50.5 K/s | 15.76 GiB | 12.09 GiB | 15.08 GiB | — | — |
| infino_rayon_default_threads | 2.09 s | 479.1 K/s | 5.45 GiB | 4.99 GiB | 5.42 GiB | -0.4% no change | **infino wins, 1.9× faster than tantivy** |
| tantivy_default_threads | 4.00 s | 249.8 K/s | 15.76 GiB | 12.09 GiB | 15.08 GiB | — | — |

<!-- END: bench/fts/superfile/ingest -->

<!-- BEGIN: bench/fts/superfile/search -->
### Superfile FTS — search (1000000 docs)

#### OR queries

| Query | infino p50 | infino Peak RSS | infino Median RSS | infino P90 RSS | infino Peak RSS Δ | Tantivy p50 | Tantivy Peak RSS | Tantivy Median RSS | Tantivy P90 RSS | Tantivy Peak RSS Δ | Winner |
|-------|------------|-----------------|-------------------|----------------|-------------------|-------------|------------------|--------------------|-----------------|--------------------|--------|

| single_rare | 466 ns | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 7.54 µs | 11.72 GiB | 9.81 GiB | 9.82 GiB | — | **infino wins, 16.2× faster than tantivy** |
| single_df1 | 218 ns | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 1.33 µs | 11.72 GiB | 9.81 GiB | 9.82 GiB | — | **infino wins, 6.1× faster than tantivy** |
| single_common | 8.97 µs | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 104.56 µs | 11.72 GiB | 9.81 GiB | 9.82 GiB | — | **infino wins, 11.7× faster than tantivy** |
| two_term_or | 151.39 µs | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 684.53 µs | 11.72 GiB | 9.81 GiB | 9.82 GiB | — | **infino wins, 4.5× faster than tantivy** |
| three_wide_or | 2.24 ms | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 4.81 ms | 11.72 GiB | 9.81 GiB | 9.82 GiB | — | **infino wins, 2.2× faster than tantivy** |
| three_similar_or | 9.00 ms | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 12.75 ms | 11.72 GiB | 9.81 GiB | 9.82 GiB | — | **infino wins, 1.4× faster than tantivy** |
| five_term_or | 17.71 ms | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 26.85 ms | 11.72 GiB | 9.81 GiB | 9.82 GiB | — | **infino wins, 1.5× faster than tantivy** |

#### AND queries

| Query | infino p50 | infino Peak RSS | infino Median RSS | infino P90 RSS | infino Peak RSS Δ | Tantivy p50 | Tantivy Peak RSS | Tantivy Median RSS | Tantivy P90 RSS | Tantivy Peak RSS Δ | Winner |
|-------|------------|-----------------|-------------------|----------------|-------------------|-------------|------------------|--------------------|-----------------|--------------------|--------|
| two_term_and | 176.63 µs | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 9.27 ms | 11.72 GiB | 9.81 GiB | 9.82 GiB | — | **infino wins, 52.5× faster than tantivy** |
| three_wide_and | 3.38 ms | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 5.29 ms | 11.72 GiB | 9.81 GiB | 9.82 GiB | — | **infino wins, 1.6× faster than tantivy** |
| three_similar_and | 5.34 ms | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 7.20 ms | 11.72 GiB | 9.81 GiB | 9.82 GiB | — | **infino wins, 1.3× faster than tantivy** |
| five_term_and | 6.54 ms | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 7.06 ms | 11.72 GiB | 9.81 GiB | 9.82 GiB | — | **infino wins, 1.1× faster than tantivy** |

#### Per-algorithm probes (infino-only, WAND+BMW vs MaxScore+BMM)

| Shape | WAND+BMW p50 | WAND+BMW Peak RSS | WAND+BMW Median RSS | WAND+BMW P90 RSS | WAND+BMW Peak RSS Δ | MaxScore+BMM p50 | MaxScore+BMM Peak RSS | MaxScore+BMM Median RSS | MaxScore+BMM P90 RSS | MaxScore+BMM Peak RSS Δ | Winner |
|-------|--------------|-------------------|---------------------|------------------|---------------------|------------------|-----------------------|-------------------------|----------------------|-------------------------|--------|
| wide_3_or | 7.20 ms | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 2.21 ms | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | **MaxScore+BMM wins, 3.3× faster than WAND+BMW** |
| similar_3_or | 13.20 ms | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 8.98 ms | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | **MaxScore+BMM wins, 1.5× faster than WAND+BMW** |
| similar_5_or | 38.23 ms | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | 17.73 ms | 3.96 GiB | 3.84 GiB | 3.86 GiB | +1.2% no change | **MaxScore+BMM wins, 2.2× faster than WAND+BMW** |

<!-- END: bench/fts/superfile/search -->

### FTS — supertable (multi-superfile, 10M docs)

<!-- BEGIN: bench/fts/supertable/ingest -->
### Supertable FTS — ingest (10000000 docs, Zipfian, 200 tokens/doc, 10K vocab)

| Engine | Time | Throughput | Peak RSS | Median RSS | P90 RSS | Peak RSS Δ | vs Tantivy |
|--------|------|------------|----------|------------|---------|------------|------------|
| infino_auto_writer_pool | 50.84 s | 196.7 K/s | 28.48 GiB | 24.27 GiB | 26.68 GiB | -11.2% improved | **tantivy wins, 1.4× faster than infino** |
| tantivy_default_threads | 37.17 s | 269.1 K/s | 49.75 GiB | 47.74 GiB | 48.75 GiB | — | — |

*Output cardinality: infino emits `min(writer_pool.threads, total_rows)` superfiles per commit (auto = cpus/2). Tantivy emits one segment per internal worker thread per commit (≈ 8 × N_chunks segments with NoMergePolicy). Override the infino writer-thread count with `INFINO_SUPERTABLE__WRITER_THREADS=N` to match Tantivy's effective output segment count.*

<!-- END: bench/fts/supertable/ingest -->

<!-- BEGIN: bench/fts/supertable/search -->
### Supertable FTS — search (10000000 docs)

| Query | infino p50 | infino Peak RSS | infino Median RSS | infino P90 RSS | infino Peak RSS Δ | Tantivy p50 | Tantivy Peak RSS | Tantivy Median RSS | Tantivy P90 RSS | Tantivy Peak RSS Δ | Winner |
|-------|------------|-----------------|-------------------|----------------|-------------------|-------------|------------------|--------------------|-----------------|--------------------|--------|
| single_rare | 72.85 µs | 25.92 GiB | 25.06 GiB | 25.07 GiB | -12.3% improved | 44.02 µs | 46.03 GiB | 46.02 GiB | 46.02 GiB | — | **tantivy wins, 1.7× faster than infino** |
| single_common | 85.62 µs | 25.92 GiB | 25.06 GiB | 25.07 GiB | -12.3% improved | 5.03 ms | 46.03 GiB | 46.02 GiB | 46.02 GiB | — | **infino wins, 58.7× faster than tantivy** |
| two_term_or | 493.32 µs | 25.92 GiB | 25.06 GiB | 25.07 GiB | -12.3% improved | 1.27 ms | 46.03 GiB | 46.02 GiB | 46.02 GiB | — | **infino wins, 2.6× faster than tantivy** |
| three_wide_or | 4.74 ms | 25.92 GiB | 25.06 GiB | 25.07 GiB | -12.3% improved | 9.85 ms | 46.03 GiB | 46.02 GiB | 46.02 GiB | — | **infino wins, 2.1× faster than tantivy** |
| three_similar_or | 16.37 ms | 25.92 GiB | 25.06 GiB | 25.07 GiB | -12.3% improved | 21.57 ms | 46.03 GiB | 46.02 GiB | 46.02 GiB | — | **infino wins, 1.3× faster than tantivy** |
| five_term_or | 35.88 ms | 25.92 GiB | 25.06 GiB | 25.07 GiB | -12.3% improved | 49.62 ms | 46.03 GiB | 46.02 GiB | 46.02 GiB | — | **infino wins, 1.4× faster than tantivy** |
| prefix | 80.02 ms | 25.92 GiB | 25.06 GiB | 25.07 GiB | -12.3% improved | 102.56 ms | 46.03 GiB | 46.02 GiB | 46.02 GiB | — | **infino wins, 1.3× faster than tantivy** |

<!-- END: bench/fts/supertable/search -->

### Vector — superfile (single-segment, 1M × dim=384)

<!-- BEGIN: bench/vector/superfile/ingest -->
### Superfile vector — ingest (1000000 docs × dim=384, Gaussian planted clusters, cosine)

| Engine | Time | Throughput | Peak RSS | Median RSS | P90 RSS | Peak RSS Δ | vs LanceDB |
|--------|------|------------|----------|------------|---------|------------|------------|
| infino | 23.69 s | 42.2 K/s | 4.07 GiB | 2.71 GiB | 3.68 GiB | -61.8% improved | **infino wins, 3.5× faster than lance** |
| lance | 82.17 s | 12.2 K/s | 5.78 GiB | 2.64 GiB | 3.12 GiB | — | — |

<!-- END: bench/vector/superfile/ingest -->

<!-- BEGIN: bench/vector/superfile/search -->
### Superfile vector — search (1000000 docs × dim=384, calibrated at recall targets)

| Recall target | infino p50 | infino Peak RSS | infino Median RSS | infino P90 RSS | infino Peak RSS Δ | Lance (probe, refine) | Lance p50 | Lance Peak RSS | Lance Median RSS | Lance P90 RSS | Lance Peak RSS Δ | Winner |
|---------------|------------|-----------------|-------------------|----------------|-------------------|-----------------------|-----------|----------------|------------------|---------------|------------------|--------|
| 0.90 | 400.72 µs | 3.75 GiB | 3.30 GiB | 3.30 GiB | -45.8% improved | (p=5, r=256) | 14.46 ms | 3.60 GiB | 3.48 GiB | 3.49 GiB | — | **infino wins, 36.1× faster than lance** |
| 0.95 | 540.68 µs | 3.75 GiB | 3.30 GiB | 3.30 GiB | -45.8% improved | (p=5, r=256) | 14.49 ms | 3.60 GiB | 3.48 GiB | 3.49 GiB | — | **infino wins, 26.8× faster than lance** |
| 0.99 | 415.60 µs | 6.92 GiB | 5.43 GiB | 5.44 GiB | +0.3% no change | (p=5, r=256) | 14.43 ms | 3.60 GiB | 3.48 GiB | 3.49 GiB | — | **infino wins, 34.7× faster than lance** |

**infino default options** (`nprobe=8, rerank_mult=20` — user-facing latency baseline):

| Metric | Value |
|--------|-------|
| infino_default_options_top10 | 336.89 µs |
| infino_default_options_top10_peak_rss | 3.75 GiB |
| infino_default_options_top10_median_rss | 3.30 GiB |
| infino_default_options_top10_p90_rss | 3.30 GiB |
| infino_default_options_top10_peak_rss_delta | -45.8% improved |

<!-- END: bench/vector/superfile/search -->

### Vector — supertable (multi-superfile, 10M × dim=384)

<!-- BEGIN: bench/vector/supertable/ingest -->
### Supertable combined FTS + vector — ingest (10000000 docs × dim=384)

Both engines build one table with a text/FTS index and a vector index before timing stops.

| Engine | Time | Throughput | Peak RSS | Median RSS | P90 RSS | Peak RSS Δ | vs LanceDB |
|--------|------|------------|----------|------------|---------|------------|------------|
| supertable | 302.30 s | 33.1 K/s | 26.87 GiB | 23.39 GiB | 25.16 GiB | -36.1% improved | **lance wins, 1.1× faster than infino** |
| lance | 282.78 s | 35.4 K/s | 37.19 GiB | 21.40 GiB | 36.98 GiB | — | — |

<!-- END: bench/vector/supertable/ingest -->

<!-- BEGIN: bench/vector/supertable/search -->
### Supertable vector — search (10000000 docs × dim=384, calibrated at recall targets)

| Recall target | supertable p50 | supertable Peak RSS | supertable Median RSS | supertable P90 RSS | supertable Peak RSS Δ | Lance (probe, refine) | Lance p50 | Lance Peak RSS | Lance Median RSS | Lance P90 RSS | Lance Peak RSS Δ | Winner |
|---------------|----------------|---------------------|-----------------------|--------------------|-----------------------|-----------------------|-----------|----------------|------------------|---------------|------------------|--------|
| 0.90 | 3.23 ms | 30.63 GiB | 30.62 GiB | 30.62 GiB | -28.2% improved | (p=25, r=256) | 79.25 ms | 28.91 GiB | 28.85 GiB | 28.88 GiB | — | **supertable wins, 24.6× faster than lance** |
| 0.95 | 3.74 ms | 30.63 GiB | 30.62 GiB | 30.62 GiB | -28.2% improved | (p=25, r=256) | 79.04 ms | 28.91 GiB | 28.85 GiB | 28.88 GiB | — | **supertable wins, 21.1× faster than lance** |
| 0.99 | 3.82 ms | 42.66 GiB | 42.64 GiB | 42.65 GiB | -4.7% no change | (p=50, r=256) | 110.45 ms | 28.91 GiB | 28.85 GiB | 28.88 GiB | — | **supertable wins, 28.9× faster than lance** |

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
