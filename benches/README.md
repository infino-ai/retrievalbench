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

| Engine                       | Time       | Throughput | vs Tantivy        |
|------------------------------|------------|------------|-------------------|
| infino_1thread               | 9.20 s     | 108.7 K/s  | **infino wins, 1.5× faster than tantivy**   |
| tantivy_1thread              | 13.42 s    | 74.5 K/s   | —                 |
| infino_rayon_default_threads | 808.95 ms  | 1.24 M/s   | **infino wins, 2.3× faster than tantivy**   |
| tantivy_default_threads      | 1.83 s     | 547.8 K/s  | —                 |

<!-- END: bench/fts/superfile/ingest -->

<!-- BEGIN: bench/fts/superfile/search -->
### Superfile FTS — search (1000000 docs)

| Query          | infino     | Tantivy    | Winner                |
|----------------|------------|------------|-----------------------|
| single_rare    | 267 ns     | 10.42 µs   | **infino wins, 39.1× faster than tantivy**      |
| single_df1     | 118 ns     | 5.98 µs    | **infino wins, 50.6× faster than tantivy**      |
| single_common  | 5.16 µs    | 74.95 µs   | **infino wins, 14.5× faster than tantivy**      |
| two_term_or    | 103.20 µs  | 427.04 µs  | **infino wins, 4.1× faster than tantivy**       |
| three_wide     | 1.25 ms    | 3.06 ms    | **infino wins, 2.5× faster than tantivy**       |
| three_similar  | 5.67 ms    | 8.05 ms    | **infino wins, 1.4× faster than tantivy**       |
| five_term      | 10.07 ms   | 18.21 ms   | **infino wins, 1.8× faster than tantivy**       |

**Per-algorithm probes** (infino-only, WAND+BMW vs MaxScore+BMM):

| Shape         | WAND+BMW   | MaxScore+BMM | Winner                |
|---------------|------------|--------------|-----------------------|
| wide_3        | 4.75 ms    | 1.24 ms      | **BMM wins, 3.8× faster than WAND+BMW**          |
| similar_3     | 9.01 ms    | 5.69 ms      | **BMM wins, 1.6× faster than WAND+BMW**          |
| similar_5     | 25.67 ms   | 9.97 ms      | **BMM wins, 2.6× faster than WAND+BMW**          |

<!-- END: bench/fts/superfile/search -->

### FTS — supertable (multi-superfile, 10M docs)

<!-- BEGIN: bench/fts/supertable/ingest -->
### Supertable FTS — ingest (10000000 docs, Zipfian, 200 tokens/doc, 10K vocab)

| Engine                  | Time       | Throughput | vs Tantivy        |
|-------------------------|------------|------------|-------------------|
| infino_auto_writer_pool | 42.63 s    | 234.6 K/s  | **tantivy wins, 1.8× faster than infino**  |
| tantivy_default_threads | 23.71 s    | 421.8 K/s  | —                 |

*Output cardinality: infino emits `min(writer_pool.threads, total_rows)` superfiles per commit (auto = cpus/2). Tantivy emits one segment per internal worker thread per commit (≈ 8 × N_chunks segments with NoMergePolicy). Override the infino writer-thread count with `INFINO_SUPERTABLE__WRITER_THREADS=N` to match Tantivy's effective output segment count.*

<!-- END: bench/fts/supertable/ingest -->

<!-- BEGIN: bench/fts/supertable/search -->
### Supertable FTS — search (10000000 docs)

| Query          | infino     | Tantivy    | Winner                |
|----------------|------------|------------|-----------------------|
| single_rare    | 42.33 µs   | 212.53 µs  | **infino wins, 5.0× faster than tantivy**       |
| single_common  | 57.16 µs   | 2.40 ms    | **infino wins, 41.9× faster than tantivy**      |
| two_term_or    | 297.03 µs  | 825.50 µs  | **infino wins, 2.8× faster than tantivy**       |
| three_wide     | 2.19 ms    | 4.95 ms    | **infino wins, 2.3× faster than tantivy**       |
| three_similar  | 7.92 ms    | 10.82 ms   | **infino wins, 1.4× faster than tantivy**       |
| five_term      | 14.29 ms   | 23.76 ms   | **infino wins, 1.7× faster than tantivy**       |
| prefix         | 32.67 ms   | 44.44 ms   | **infino wins, 1.4× faster than tantivy**       |

<!-- END: bench/fts/supertable/search -->

### Vector — superfile (single-segment, 1M × dim=384)

<!-- BEGIN: bench/vector/superfile/ingest -->
### Superfile vector — ingest (1000000 docs × dim=384, Gaussian planted clusters, cosine)

| Engine | Time | Throughput | vs LanceDB |
|--------|------|------------|------------|
| infino | 18.92 s | 52.9 K/s | **infino wins, 1.5× faster than lance** |
| lance | 28.13 s | 35.6 K/s | — |

<!-- END: bench/vector/superfile/ingest -->

<!-- BEGIN: bench/vector/superfile/search -->
### Superfile vector — search (1000000 docs × dim=384, calibrated at recall targets)

| Recall target | infino (probe, refine) | infino p50 | Lance (probe, refine) | Lance p50 | Winner |
|---------------|------------------------|------------|-----------------------|-----------|--------|
| 0.90 | (p=1, r=1024) | 4.94 ms | (p=10, r=256) | 9.21 ms | **infino wins, 1.9× faster than lance** |
| 0.95 | (p=5, r=1024) | 5.02 ms | (p=10, r=256) | 9.56 ms | **infino wins, 1.9× faster than lance** |
| 0.99 | (p=5, r=1024) | 5.03 ms | (p=10, r=256) | 9.17 ms | **infino wins, 1.8× faster than lance** |

**infino default options** (`nprobe=8, rerank_mult=20` — user-facing latency baseline):

| Metric | Value |
|--------|-------|
| infino_default_options_top10 | 4.97 ms |

<!-- END: bench/vector/superfile/search -->

### Vector — supertable (multi-superfile, 10M × dim=384)

<!-- BEGIN: bench/vector/supertable/ingest -->
### Supertable vector — ingest (10000000 docs × dim=384, sharded into 4 superfiles)

| Engine | Time | Throughput | vs LanceDB |
|--------|------|------------|------------|
| supertable | 310.17 s | 32.2 K/s | **lance wins, 3.1× faster than infino** |
| lance | 99.94 s | 100.1 K/s | — |

<!-- END: bench/vector/supertable/ingest -->

<!-- BEGIN: bench/vector/supertable/search -->
### Supertable vector — search (10000000 docs × dim=384, calibrated at recall targets)

| Recall target | supertable (probe/seg, refine) | supertable p50 | Lance (probe, refine) | Lance p50 | Winner |
|---------------|--------------------------------|----------------|-----------------------|-----------|--------|
| 0.90 | (p=1, r=4) | 56.07 ms | (p=10, r=256) | 29.08 ms | **lance wins, 1.9× faster than supertable** |
| 0.95 | (p=12, r=4) | 56.06 ms | (p=25, r=256) | 35.67 ms | **lance wins, 1.6× faster than supertable** |
| 0.99 | (p=16, r=4) | 55.94 ms | (p=50, r=256) | 44.57 ms | **lance wins, 1.3× faster than supertable** |

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
