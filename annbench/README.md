# annbench — ann-benchmarks recall harness for infino

Reproducible recall@10 benchmarks for infino on the standard
[ann-benchmarks](https://github.com/erikbern/ann-benchmarks) datasets — the ones
VDBBench does **not** ship ground truth for. Covers all three infino metrics:
**L2** (`euclidean`), **cosine** (`angular`), and **negdot** (`dot`).

## What it does

`annbench_infino.py` reads each dataset's HDF5 (`train` / `test` / `neighbors`),
ingests `train` into a local infino table, runs every `test` query, and scores
recall@10 against the precomputed `neighbors` ground truth — same methodology as
VDBBench (k=10, serial). Angular datasets are unit-normalized for infino's fixed
cosine grid (the cosine-similarity ranking, and therefore the GT, is preserved).

## Datasets (metric derived from the name suffix)

| suffix | infino metric | datasets |
|---|---|---|
| `-euclidean` | `l2sq` | sift-128, gist-960, mnist-784, fashion-mnist-784 |
| `-angular` | `cosine` | glove-25 / 50 / 100 / 200, nytimes-256, deep-image-96 |
| `-dot` | `negdot` | lastfm-64 |

Jaccard datasets (e.g. kosarak) are excluded — infino has no jaccard metric.

## Run

Prereqs: a Python venv with the published `infino` wheel plus `h5py pyarrow numpy`.

```bash
export PYTHON=/path/to/venv/bin/python
./run.sh <infino-catalog-dir> <hdf5-data-dir> [dataset ...]
# e.g. the whole roster:
./run.sh /data/bench-vdb/ann-infino /data/annbench-data
# or a subset:
./run.sh /data/bench-vdb/ann-infino /data/annbench-data sift-128-euclidean gist-960-euclidean
```

Datasets auto-download from `ann-benchmarks.com` (the harness sends a browser
User-Agent — the host 403s the default urllib one). Each dataset reports
recall@10 at the stamped default and at `rerank_mult=256`.

## Findings snapshot (latest infino main)

Full results in infino#490. Across 11 datasets / 3 metrics the stamped default
lands ~0.97–0.99, with two known gaps: **glove-25** (very low dim) and **Bioasq**
(question-vs-passage calibration, infino#515). Forcing `rerank_mult=256` generally
*hurts* recall (it narrows the coupled width law) — trust the default.
