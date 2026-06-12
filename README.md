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

Scale knobs are inherited from `infino-bench-utils`:

- `INFINO_BENCH_SUPERFILE_DOCS`
- `INFINO_BENCH_SUPERTABLE_DOCS`
- `INFINO_BENCH_WRITERS`

## Object-store backends

Both engines honor the same selection infino's bench uses — explicit,
never inferred from credentials:

| `INFINO_BENCH_STORE` | Extra env | Lance peer location |
|---|---|---|
| `s3` | `INFINO_REAL_S3_BUCKET` (+ AWS creds/region) | `s3://<bucket>/retrievalbench-lance/...` |
| `azure` | `INFINO_REAL_AZURE_CONTAINER` + `AZURE_STORAGE_ACCOUNT_NAME`/`_KEY` | `az://<container>/retrievalbench-lance/...` |
| `s3s_fs` (default) | — | supertable comparison skipped (real store required) |

Report columns are labeled by what was measured: `lancedb-s3` / `lancedb-azure`.

### Adding an object store

Every store is one row of data in
`benches/comparison/engines/lance/location.rs` (`STORES`) plus the matching
lancedb cargo feature — nothing else changes. GCS, for example:

1. `Cargo.toml`: `lancedb = { version = "0.30.0", features = ["aws", "azure", "gcs"] }`
2. Append to `STORES`:

   ```rust
   StoreSpec {
       token: "gcs",
       label: "lancedb-gcs",
       scheme: "gs",
       container_envs: &["INFINO_REAL_GCS_BUCKET", "INFINO_TEST_REAL_GCS_BUCKET"],
       prefix_env: "INFINO_REAL_GCS_PREFIX",
       cred_envs: &[("GOOGLE_SERVICE_ACCOUNT", "google_service_account")],
   },
   ```

3. `cargo test --lib lance::location` — the `every_spec_row_is_well_formed`
   invariants gate the new row.
4. Live-smoke one cell on the new store.

Infino-side support for the same store is a prerequisite for the comparison
to run end-to-end — `INFINO_BENCH_STORE` gates both engines.

## Bench Layout

```text
benches/comparison/main.rs        unified comparison selector
benches/comparison/superfile.rs   Infino vs Lance/Tantivy, in-memory tier
benches/comparison/supertable.rs  Infino vs LanceDB remote, object-store tier
benches/comparison/engines/       peer engine adapters
```
