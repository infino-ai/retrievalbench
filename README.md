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

```sh
# Run every current comparison.
cargo bench --bench comparison

# Run one or more superfile comparisons.
cargo bench --bench comparison -- superfile_fts
cargo bench --bench comparison -- superfile_vector superfile_sql

# Run one or more supertable comparisons.
cargo bench --bench comparison -- supertable_fts hot
cargo bench --bench comparison -- supertable_vector cold
cargo bench --bench comparison -- supertable_sql build hot cold
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
