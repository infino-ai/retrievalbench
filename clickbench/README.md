# ClickBench results

infino's ClickBench numbers against the published reference engines, on the ClickBench reference machine **c6a.4xlarge** at the full 100M-row scale.

Across the full self-hosted ClickBench field on c6a.4xlarge, infino ranks **#23 of 94** engines by hot-run total, and beats every general-purpose engine reading Parquet except DuckDB, Polars, and CedarDB. See the [full comparison](FULL_COMPARISON.md) for all 94.

This README keeps the headline comparison against the two engines that matter most for us, DataFusion and ClickHouse. infino's result file is stored here; the reference numbers link to their source on upstream ClickBench.

## Results (c6a.4xlarge, 100M rows)

| System | Cold sum | Cold geomean | Hot sum | Hot geomean |
|---|--:|--:|--:|--:|
| [**infino**](results/infino/c6a.4xlarge.json) | 926.29s * | 20.79s * | **38.46s** | 0.3425s |
| [DataFusion (Parquet, single)](results/datafusion/c6a.4xlarge.json) | 185.82s | 1.22s | 45.92s | 0.3556s |
| [ClickHouse (Parquet, single)](results/clickhouse-parquet/c6a.4xlarge.json) | 198.14s | 1.33s | 48.05s | 0.4264s |
| [ClickHouse (native, MergeTree)](results/clickhouse/c6a.4xlarge.json) | 154.79s | 1.58s | 32.26s | 0.1306s |

On hot, infino beats DataFusion and ClickHouse-on-Parquet. It trails ClickHouse's native MergeTree, which is a different substrate (ClickHouse ingests into its own format rather than reading Parquet).

### How ClickBench measures

Each query is run three times. **Cold** is the first run (`t1`); **hot** is the best of the warm runs (`min(t2, t3)`). **Sum** is the total across all 43 queries; **geomean** is the geometric mean, so no single slow query dominates. Lower is better everywhere.

### \* Ignore infino's cold numbers for now

infino's cold figures are preliminary and should be disregarded at this stage:

1. **Not measured the same way.** infino's cold was taken with the OS page cache dropped before every query, so each `t1` is a true cold-storage read. The upstream reference cold numbers run against a warm OS cache (data already resident from load). The two are not comparable.
2. **Cold path still in progress.** infino currently re-opens the table and cold-fetches whole superfiles per query. That path is an active optimization target, so the cold numbers will move.

Hot is the comparable, meaningful metric today.

## Queries

The exact queries and harness used for the infino run live in our ClickBench fork on the [`add-infino`](https://github.com/infino-ai/clickbench/tree/add-infino) branch:

- infino: https://github.com/infino-ai/clickbench/blob/add-infino/infino/queries.sql
- DataFusion: https://github.com/infino-ai/clickbench/blob/add-infino/datafusion/queries.sql
- ClickHouse: https://github.com/infino-ai/clickbench/blob/add-infino/clickhouse/queries.sql

The reference engines' query files are unchanged from upstream ClickBench; only the infino system directory is ours.

## Correctness

The infino numbers are on verified-correct results. All 43 infino query outputs were row-diffed against DataFusion over the same 100M-row Parquet:

- **34 of 43 are bit-identical.**
- The other 9 differ only for benign reasons: floating-point summation order in an `AVG`; infino's `SELECT *` exposing its internal `_id` column; and queries that are inherently non-deterministic (a `LIMIT` with no total `ORDER BY`, or ties on a non-unique sort key, where any engine may return a different valid set of rows).

No query returns a wrong computation.

## Sources

infino's JSON is from our `clickbench-cloud` run on 2026-07-19: infino `main` (commit `8d45bdc1`), AWS c6a.4xlarge, 100M rows, portable build (no `target-cpu`) with fat LTO and `codegen-units = 1`, matching DataFusion's build recipe.

Reference numbers are from upstream [ClickBench](https://github.com/ClickHouse/ClickBench). The three committed here are copied verbatim from:

- DataFusion: https://github.com/ClickHouse/ClickBench/blob/main/datafusion/results/20260629/c6a.4xlarge.json
- ClickHouse (Parquet): https://github.com/ClickHouse/ClickBench/blob/main/clickhouse-parquet/results/20260624/c6a.4xlarge.json
- ClickHouse (native): https://github.com/ClickHouse/ClickBench/blob/main/clickhouse/results/20260624/c6a.4xlarge.json

The rest of the field is linked from [FULL_COMPARISON.md](FULL_COMPARISON.md) rather than copied in. Managed cloud warehouses (Snowflake, Databricks, BigQuery, Redshift) are excluded because they do not run on c6a.4xlarge.

## Updating

These files are meant to be refreshed programmatically. A later pass will add a script that turns a `clickbench-cloud` run-log artifact into `results/infino/<machine>.json`, so a new machine or a new run is one command. Reference numbers are re-read from upstream ClickBench.
