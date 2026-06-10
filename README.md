# retrievalbench comparison harness

This repo contains third-party comparison benches for Infino. The benchmark
drivers, corpora, reporting, and Infino reference engines come from
`../infino/benches/utils`; this repo only adds peer-engine adapters and
comparison entry points.

Current peer scope:

- LanceDB: FTS, vector, SQL
- Tantivy: FTS

DuckDB and CoreDB are not wired in this branch.

## Invocation

Single binary, same positional grammar as infino's own bench suite:
`cargo bench -- [tier] [modality] [phase ...]`.

```sh
# Run every current comparison.
cargo bench

# Run one tier or one cell.
cargo bench -- superfile
cargo bench -- superfile fts
cargo bench -- superfile vector sql

# Supertable cells, optionally narrowed by phase.
cargo bench -- supertable fts warm
cargo bench -- supertable vector cold
cargo bench -- supertable sql build warm cold
```

The superfile comparisons run Infino vs the in-memory peer adapters. The
supertable comparisons run Infino vs the object-store peer adapters. Unsupported
cells are omitted: Tantivy participates in FTS superfile comparisons only;
LanceDB participates in all current FTS/vector/SQL cells.

Scale and backend knobs are inherited from `infino-bench-utils`:

- `INFINO_BENCH_SUPERFILE_DOCS`
- `INFINO_BENCH_SUPERTABLE_DOCS`
- `INFINO_BENCH_WRITERS`
- `INFINO_BENCH_STORE`
- `INFINO_REAL_S3_BUCKET` / `INFINO_REAL_AZURE_CONTAINER`

## Bench Layout

```text
benches/comparison/main.rs        unified comparison selector
benches/comparison/superfile.rs   Infino vs Lance/Tantivy, in-memory tier
benches/comparison/supertable.rs  Infino vs LanceDB S3, object-store tier
benches/comparison/engines/       peer engine adapters
```
