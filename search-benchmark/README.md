# Track D — full-Wikipedia FTS (`search-benchmark-game`)

Out-of-process FTS comparison: each engine is its own HTTP server, a Python
client times queries, single-threaded, best-of-10. Corpus is English Wikipedia.
This is a different methodology from Tracks A–C and is not comparable to them
number-for-number.

Harness code stays in [`infino-ai/search-benchmark-game`](https://github.com/infino-ai/search-benchmark-game)
(it already has a nightly VM, a GitHub Pages dashboard, and a turbopuffer splice).
This directory is the retrievalbench fold-in: docs, the dispatch workflow, and
the committed `results.json` / `provenance.json` from the latest dispatched run.
Those files are added only after the first successful run.

## Published dashboard

- Comparison set (31 queries, turbopuffer splice): https://infino-ai.github.io/search-benchmark-game/
- Full 962-query set: https://infino-ai.github.io/search-benchmark-game/full

Hardware for those numbers: AWS **c7i.2xlarge** (8 vCPU, 16 GiB), chosen to
match turbopuffer's published instance.

## First-pass engines (Rust overlap with `benches/comparison/`)

`infino-main` and `tantivy-0.26`. Lucene, bleve, bluge, iresearch, pisa, and
rucene come along once the game repo is cloned on the VM — adding an engine
there never touches Infino's own build.

## Refresh

Dispatch from this repo (provisions a VM, clones the game, runs
`ENGINES="infino-main tantivy-0.26" make compile index bench`):

```sh
gh workflow run searchbenchmark-cloud.yml --repo infino-ai/retrievalbench
```

The workflow artifact contains `search-benchmark/results.json` and
`search-benchmark/provenance.json` with the resolved Infino and harness SHAs.
Download that artifact into this repository and commit both files together;
the dashboard links below remain the full-roster nightly source.

Or dispatch the game's own nightly (full engine roster, commits `results.json`
on that repo, Pages publishes):

```sh
gh workflow run nightly-bench.yml --repo infino-ai/search-benchmark-game
```
