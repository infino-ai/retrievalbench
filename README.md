# RetrievalBench

Infino's external benchmark harness. 

## The benchmarks

| Benchmark | Comparators | Scale | Where |
|---|---|---|---|
| [Embedded vector libraries](#embedded-vector-libraries) | turbovec, FAISS, LanceDB | 100K / 1M / 10M | [`benches/comparison/`](benches/comparison/) → [`results/inprocess/`](results/inprocess/) |
| [Vector databases](#vector-databases) | VectorDBBench's bundled peers | 1M / 10M | [live viewer](https://vdbbench-viewer-q6unoyyhua-uc.a.run.app) |
| [SQL on ClickBench](#sql-on-clickbench) | the public leaderboard | 100M rows | [`clickbench/`](clickbench/README.md) |
| [Full-text at Wikipedia scale](#full-text-at-wikipedia-scale) | Tantivy, Lucene, … | fixed Wikipedia corpus | [`search-benchmark/`](search-benchmark/README.md) |
| [Cost and scale](#cost-and-scale) | none — Infino's own $/query | 100K → 10M | in-process harness + committed JSON |

## Embedded vector libraries

Every engine in one process: same queries, brute-force exact ground truth,
each library reached through its own public API. Infino's row is the shipped
`flat_ivf` mode — a table built by the normal lifecycle (`append` → `commit`
→ `optimize()`), serving the resident 4-bit plane that one config line
selects.

Measured at dbpedia-1536, 100K rows, top-10 (box threads):

| engine | recall@10 | warm p50 | resident |
|---|---|---|---|
| Infino `flat_ivf` | 0.934 | 1.51 ms | 74.8 MiB |
| turbovec 4-bit | 0.944 | 1.55 ms | 75.2 MiB |
| turbovec 2-bit | 0.835 | 0.42 ms | 37.8 MiB |
| FAISS PQ fastscan | 0.672 | 4.38 ms | 74.1 MiB |
| FAISS PQ 8-bit | 0.943 | 45.3 ms | 75.5 MiB |

Reproduce that table with plain `cargo bench` (the corpus downloads once):

```sh
printf 'vector:\n  search_mode: flat_ivf\n' > infino.yaml
INFINO_BENCH_SUPERTABLE_DOCS=100000 \
  cargo bench -- vector-codec \
  corpus=hf:KShivendu/dbpedia-entities-openai-1M corpus-dir=./corpora
```

Add `--features faiss` (after `scripts/build_faiss.sh`, which builds the
exact FAISS source faiss-rs bundles, with `-march=native` — without it
FastScan silently runs scalar) for the FAISS rows. Remove `infino.yaml`
before running other cells; everything else measures shipped defaults.

To regenerate the committed results — every battery for one corpus rung,
both thread modes, publisher refusing a dirty tree and stamping
host/commit/command into `run.json`:

```sh
./scripts/publish_results.sh dbpedia-1536-100k 100000 \
  hf:KShivendu/dbpedia-entities-openai-1M ./corpora
```

Selection grammar matches Infino's own bench suite:
`cargo bench -- [tier] [modality] [phase ...]`, plus the `vector-codec` and
`table-writes` cells. Scale knobs: `INFINO_BENCH_SUPERFILE_DOCS`,
`INFINO_BENCH_SUPERTABLE_DOCS`, `INFINO_BENCH_STORE`. Engine behavior is
YAML-only; environment variables never change it.

## Vector databases

Runs through [VectorDBBench](https://github.com/zilliztech/VectorDBBench)
via [our client](https://github.com/infino-ai/VectorDBBench/tree/main/vectordb_bench/backend/clients/infino),
dispatched to a VM whose instance type is recorded per run.

Viewer (stable URL): **https://vdbbench-viewer-q6unoyyhua-uc.a.run.app**

```sh
gh workflow run vdbbench-vector.yml --repo infino-ai/retrievalbench \
  -f vector_case=Performance768D1M -f publish_results=true
```

Docs: [`vdbbench-viewer/README.md`](vdbbench-viewer/README.md).

## SQL on ClickBench

Runs through [our port](https://github.com/infino-ai/clickbench/tree/add-infino/infino)
of the public [ClickBench](https://benchmark.clickhouse.com/) suite.
Headline on c8g.metal-48xl: hot sum **6.45 s**, geomean **0.090** — #17 of
60 systems. Details and per-machine results: [`clickbench/`](clickbench/README.md).

Refresh a result from a `clickbench-cloud` log artifact:

```sh
python3 scripts/ingest_clickbench_log.py \
  --log /tmp/clickbench.log \
  --machine c8g.metal-48xl \
  --infino-ref <commit-sha> \
  --out clickbench/results/infino/c8g.metal-48xl.json
```

## Full-text at Wikipedia scale

[Search Benchmark, the Game](https://tantivy-search.github.io/bench/) via
[our fork](https://github.com/infino-ai/search-benchmark-game) —
single-threaded, full-Wikipedia, HTTP-timed. Dispatch
`searchbenchmark-cloud.yml` or use the game's own nightly; see
[`search-benchmark/README.md`](search-benchmark/README.md). Do not fold these
numbers into the in-process tables: the measurement model is different.


## Hardware

| Suite | Machine |
|---|---|
| In-process (embedded, cost) | recorded in each run's `results/inprocess/<run>/run.json`; official RSS runs require Linux |
| Vector databases | dispatched VM, instance type recorded per run |
| ClickBench | `c6a.4xlarge` (reference) and `c8g.metal-48xl` (leaderboard) |
| Full-text | AWS `c7i.2xlarge` (matches turbopuffer's published instance) |

## Comparator pins

Recorded in `Cargo.toml` / `Cargo.lock` and copied into each run's
provenance. No personal forks — every pin is a canonical-repo SHA or a
crates.io release: Infino `447ff2fc` (main `3aaffb64` plus the benches-only
commits under review as infino-ai/infino#679; the pin repoints to a main SHA
when that merges), turbovec `ccab9f32` (the 1.0.0 release), LanceDB 0.37.1,
faiss-rs 0.13.0.
