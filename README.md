# retrievalbench

One harness for Infino's external comparisons. Four tracks; each owns the
scale and corpus where the comparison is honest. These are our measurements,
from a stated commit, on a stated machine — not a third-party-reproducible
board until this repo is public.

## Tracks

| Track | What | Comparators | Scale | Where |
|---|---|---|---|---|
| **A** | In-memory libraries | FAISS, turbovec, LanceDB | 100K / 1M / 10M | `benches/comparison/` |
| **B** | Vector databases | Milvus, Qdrant, and VectorDBBench's own bundled peers | 1M / 10M / 100M | [live viewer](https://vdbbench-viewer-q6unoyyhua-uc.a.run.app) |
| **C** | Infino-only cost | none (GET count, bytes/query, $/month, cold vs warm) | 100K → 100M | in-process harness + committed JSON |
| **D** | Full-Wikipedia FTS | Tantivy, Lucene, … (out of process, HTTP-timed) | fixed Wikipedia corpus | [`search-benchmark/`](search-benchmark/README.md) |

Track A stops at 10M because RAM does. A 100M × 768-d corpus is 307 GB as
float32 and 38 GB even at 4-bit; it does not fit on the Track A/C box. Infino
serves that scale from object storage — that is Track C, and the absence of a
comparator column is the result.

Track D is single-threaded, full-Wikipedia, HTTP-timed. Do not fold those
numbers into a Track A table.

## Hardware

| Track | Machine |
|---|---|
| A / C (in-process) | recorded in every run's `results/inprocess/<run>/run.json`; official RSS runs require Linux |
| B | dispatched VM, instance type recorded per VectorDBBench run |
| ClickBench | `c6a.4xlarge` (reference) and `c8g.metal-48xl` (leaderboard) — see [`clickbench/`](clickbench/README.md) |
| D | AWS `c7i.2xlarge` (matches turbopuffer's published instance) |

## Invocation (Track A)

```sh
cargo bench -- [tier] [modality] [phase ...]
# Full declared matrix: dbpedia-1536 + glove-200 + Cohere-768,
# 100K / 1M / 10M where each corpus exists.
./scripts/run_track_a.sh

# One development cell:
INFINO_BENCH_ALLOW_DIRTY=1 ./scripts/run_track_a.sh \
  glove-200-100k 100000 annb:glove-200-angular target/corpora
```

Env: `INFINO_BENCH_SUPERFILE_DOCS`, `INFINO_BENCH_SUPERTABLE_DOCS`,
`INFINO_BENCH_STORE`.

The runner builds the exact FAISS source bundled by faiss-rs, refuses
official publication from a dirty tree, preserves each corpus/scale under a
separate results directory, and regenerates [`results/README.md`](results/README.md).

Track C's 100M object-store cost row is deferred. Historical mixed-commit
numbers are not copied into this battery.

## Track B (VectorDBBench)

Viewer (stable URL): **https://vdbbench-viewer-q6unoyyhua-uc.a.run.app**

```sh
gh workflow run vdbbench-vector.yml --repo infino-ai/retrievalbench \
  -f vector_case=Performance768D1M -f publish_results=true
gh workflow run vdbbench-vector.yml --repo infino-ai/retrievalbench \
  -f vector_case=Performance768D10M -f publish_results=true
```

Docs: [`vdbbench-viewer/README.md`](vdbbench-viewer/README.md).

## ClickBench

Headline (c8g.metal-48xl): hot sum **6.45 s**, geomean **0.090**, #17 of 60.
See [`clickbench/README.md`](clickbench/README.md).

Refresh from a `clickbench-cloud` log artifact:

```sh
python3 scripts/ingest_clickbench_log.py \
  --log /tmp/clickbench.log \
  --machine c8g.metal-48xl \
  --infino-ref <commit-sha> \
  --out clickbench/results/infino/c8g.metal-48xl.json
```

## Track D

[`search-benchmark/README.md`](search-benchmark/README.md) — dispatch
`searchbenchmark-cloud.yml`, or the game's own nightly.

## Comparator pins

Recorded in `Cargo.toml` / `Cargo.lock` and copied into each run's provenance.
No personal forks. Infino is canonical SHA `23bb9cca`; LanceDB 0.37.1,
turbovec `ccab9f32`, and faiss-rs 0.13.0.
