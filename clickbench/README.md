# ClickBench results

infino's ClickBench numbers against the published reference engines, on the ClickBench reference machine **c6a.4xlarge** at the full 100M-row scale.

Across the full self-hosted ClickBench field on c6a.4xlarge, infino ranks **#21 of 94** engines by hot-run total, and beats every general-purpose engine reading Parquet except DuckDB and CedarDB. See the [full comparison](FULL_COMPARISON.md) for all 94.

This README keeps the headline comparison against the two engines that matter most for us, DataFusion and ClickHouse. infino's result file is stored here; the reference numbers link to their source on upstream ClickBench.

## Results (c6a.4xlarge, 100M rows)

| System | Cold sum | Cold geomean | Hot sum | Hot geomean |
|---|--:|--:|--:|--:|
| [**infino**](results/infino/c6a.4xlarge.json) | 603.36s * | 8.34s * | **35.66s** | 0.2953s |
| [DataFusion (Parquet, single)](results/datafusion/c6a.4xlarge.json) | 185.82s | 1.22s | 45.92s | 0.3556s |
| [ClickHouse (Parquet, single)](results/clickhouse-parquet/c6a.4xlarge.json) | 198.14s | 1.33s | 48.05s | 0.4264s |
| [ClickHouse (native, MergeTree)](results/clickhouse/c6a.4xlarge.json) | 154.79s | 1.58s | 32.26s | 0.1306s |

On hot, infino beats DataFusion and ClickHouse-on-Parquet. It trails ClickHouse's native MergeTree, which is a different substrate (ClickHouse ingests into its own format rather than reading Parquet).

## The leaderboard machine (c8g.metal-48xl, 100M rows)

The numbers quoted off the ClickBench homepage come from the largest machine, **c8g.metal-48xl** (Graviton4, 192 vCPU), not the c6a.4xlarge reference above. infino now has a result there too: **hot sum 6.45s, geomean 0.090** ([result file](results/infino/c8g.metal-48xl.json)), the fastest of five clean board-standard three-try sweeps on an idle box. Full detail and the AMD c7a.metal number are in [MACHINE_SCALING.md](MACHINE_SCALING.md).

Full field on this machine, best hot sum per system (60 systems). infino ranks **#17**, ahead of every DataFusion build (including the Vortex-partitioned one) and every ClickHouse-on-Parquet variant; everything ahead is a native-format or in-memory engine, or a DuckDB datalake variant.

| # | System | Hot sum |
|--:|---|--:|
| 1 | umbra | 1.61s |
| 2 | cedardb | 2.80s |
| 3 | duckdb | 3.01s |
| 4 | gizmosql | 3.18s |
| 5 | clickhouse | 3.69s |
| 6 | duckdb-memory | 3.72s |
| 7 | clickhouse-web | 3.84s |
| 8 | pg_clickhouse | 4.00s |
| 9 | firebolt | 4.48s |
| 10 | polars-dataframe | 4.73s |
| 11 | starrocks | 5.31s |
| 12 | duckdb-parquet-partitioned | 5.42s |
| 13 | arc | 5.55s |
| 14 | polars | 5.91s |
| 15 | duckdb-parquet | 6.19s |
| 16 | duckdb-datalake | 6.41s |
| **17** | **infino** | **6.45s** |
| 18 | duckdb-datalake-partitioned | 6.52s |
| 19 | datafusion-vortex-partitioned | 6.60s |
| 20 | duckdb-dataframe | 7.02s |
| 21 | datafusion-partitioned | 7.27s |
| 22 | chdb | 8.06s |
| 23 | datafusion | 8.54s |
| 24 | clickhouse-datalake-partitioned | 8.73s |
| 25 | clickhouse-parquet-partitioned | 8.93s |
| 26 | cedardb-parquet | 9.79s |
| 27 | clickhouse-parquet | 10.62s |
| 28 | clickhouse-datalake | 13.35s |
| 29 | sail | 13.57s |
| 30 | chdb-parquet-partitioned | 13.99s |
| 31 | sail-partitioned | 14.15s |
| 32 | chdb-dataframe | 15.57s |
| 33 | victorialogs | 18.17s |
| 34 | firebolt-parquet-partitioned | 18.58s |
| 35 | pg_duckdb-parquet | 21.70s |
| 36 | duckdb-vortex | 29.08s |
| 37 | datafusion-vortex | 29.84s |
| 38 | daft-parquet | 31.11s |
| 39 | bemidb | 35.78s |
| 40 | daft-parquet-partitioned | 44.00s |
| 41 | glaredb-partitioned | 44.22s |
| 42 | glaredb | 46.36s |
| 43 | spark-comet | 61.04s |
| 44 | gendb | 70.06s |
| 45 | trino-partitioned | 85.23s |
| 46 | trino | 90.61s |
| 47 | trino-datalake-partitioned | 97.42s |
| 48 | trino-datalake | 105.07s |
| 49 | firebolt-parquet | 114.44s |
| 50 | presto-partitioned | 122.40s |
| 51 | cloudberry | 127.81s |
| 52 | warehousepg | 134.12s |
| 53 | presto-datalake-partitioned | 136.56s |
| 54 | presto | 152.19s |
| 55 | presto-datalake | 159.73s |
| 56 | spark | 300.89s |
| 57 | timescaledb | 495.27s |
| 58 | cratedb | 693.13s |
| 59 | greenplum | 738.17s |
| 60 | bqn | 1447.20s |

Reference rows are best-per-system from upstream ClickBench's `c8g.metal-48xl` results; the infino row is ours. Managed warehouses (Snowflake, Databricks, BigQuery, Redshift) are excluded because they do not run on a fixed instance.

### How ClickBench measures

Each query is run three times. **Cold** is the first run (`t1`); **hot** is the best of the warm runs (`min(t2, t3)`). **Sum** is the total across all 43 queries; **geomean** is the geometric mean, so no single slow query dominates. Lower is better everywhere.

### \* Ignore infino's cold numbers for now

infino's cold figures are preliminary and should be disregarded at this stage:

1. **Not measured the same way.** infino's cold was taken with the OS page cache dropped before every query, so each `t1` is a true cold-storage read. The upstream reference cold numbers run against a warm OS cache (data already resident from load). The two are not comparable.
2. **Cold path still improving.** Cross-process cache reuse landed (a fresh process rebuilds its cache index from files a prior process left on disk instead of re-fetching from source) and already cut cold sharply: cold sum from 926s to 610s, cold geomean from 20.8s to 8.3s between runs. It is still an active optimization target, so the numbers will keep moving.

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

infino's JSON is from our `clickbench-cloud` run on 2026-07-21: infino `main` (commit `aa3a6247`), AWS c6a.4xlarge, 100M rows, 32 GiB disk cache, portable build (no `target-cpu`) with fat LTO and `codegen-units = 1`, matching DataFusion's build recipe.

Reference numbers are from upstream [ClickBench](https://github.com/ClickHouse/ClickBench). The three committed here are copied verbatim from:

- DataFusion: https://github.com/ClickHouse/ClickBench/blob/main/datafusion/results/20260629/c6a.4xlarge.json
- ClickHouse (Parquet): https://github.com/ClickHouse/ClickBench/blob/main/clickhouse-parquet/results/20260624/c6a.4xlarge.json
- ClickHouse (native): https://github.com/ClickHouse/ClickBench/blob/main/clickhouse/results/20260624/c6a.4xlarge.json

The rest of the field is linked from [FULL_COMPARISON.md](FULL_COMPARISON.md) rather than copied in. Managed cloud warehouses (Snowflake, Databricks, BigQuery, Redshift) are excluded because they do not run on c6a.4xlarge.

## Updating

These files are meant to be refreshed programmatically. A later pass will add a script that turns a `clickbench-cloud` run-log artifact into `results/infino/<machine>.json`, so a new machine or a new run is one command. Reference numbers are re-read from upstream ClickBench.
