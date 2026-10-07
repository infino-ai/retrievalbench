# ClickBench results

infino's ClickBench numbers against the published reference engines, on the ClickBench reference machine **c6a.4xlarge** at the full 100M-row scale.

Across the full self-hosted ClickBench field on c6a.4xlarge, infino ranks **#39 of 118** engines by hot-run total. See the [full comparison](FULL_COMPARISON.md) for all 118.

This README keeps the headline comparison against the two engines that matter most for us, DataFusion and ClickHouse. All numbers are from upstream [ClickBench](https://github.com/ClickHouse/ClickBench), where infino now has published results; each row links to its folder there, and the result files are mirrored under `results/`.

## Results (c6a.4xlarge, 100M rows)

| System | Cold sum | Cold geomean | Hot sum | Hot geomean |
|---|--:|--:|--:|--:|
| [**infino**](https://github.com/ClickHouse/ClickBench/tree/main/infino) ([file](results/infino/c6a.4xlarge.json)) | 128.46s | 1.891s | **33.74s** | **0.2636s** |
| [DataFusion (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion) ([file](results/datafusion/c6a.4xlarge.json)) | 182.91s | 1.169s | 45.57s | 0.3558s |
| [ClickHouse (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet) ([file](results/clickhouse-parquet/c6a.4xlarge.json)) | 127.72s | 1.247s | 28.39s | 0.3218s |
| [ClickHouse (native, MergeTree)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse) ([file](results/clickhouse/c6a.4xlarge.json)) | 113.72s | 1.071s | 17.53s | 0.1022s |

On hot, infino beats DataFusion on both total and geomean. Against ClickHouse-on-Parquet the two are close: infino is behind on hot total (33.74s vs 28.39s) but ahead on hot geomean (0.2636 vs 0.3218) — one slow query weighs on infino's total while its per-query distribution is tighter. ClickHouse's native MergeTree is faster on both, on a different substrate (it ingests into its own format rather than reading Parquet). On cold, infino now sits in the same band as ClickHouse-on-Parquet (128.46s vs 127.72s) and ahead of DataFusion (182.91s).

## The leaderboard machine (c8g.metal-48xl, 100M rows)

The numbers quoted off the ClickBench homepage come from the largest machine, **c8g.metal-48xl** (Graviton4, 192 vCPU), not the c6a.4xlarge reference above. infino's result there: **hot sum 6.04s, geomean 0.0837** ([result file](results/infino/c8g.metal-48xl.json)).

Full field on this machine, best hot sum per system (82 systems). infino ranks **#20**, ahead of every DataFusion build and every ClickHouse-on-Parquet variant. Ahead of it sit the native-format and in-memory engines, plus a handful of newer Parquet/dataframe readers (Pivot, Umbra-on-Parquet, Polars, DuckDB-Parquet-partitioned).

| # | System | Hot sum |
|--:|---|--:|
| 1 | [elosdb](https://github.com/ClickHouse/ClickBench/tree/main/elosdb) | 0.87s |
| 2 | [Pivotlake (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/pivot-parquet-partitioned) | 1.03s |
| 3 | [Pivotlake (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/pivot-parquet) | 1.05s |
| 4 | [Umbra](https://github.com/ClickHouse/ClickBench/tree/main/umbra) | 1.17s |
| 5 | [Firebolt](https://github.com/ClickHouse/ClickBench/tree/main/firebolt) | 2.32s |
| 6 | [CedarDB](https://github.com/ClickHouse/ClickBench/tree/main/cedardb) | 2.38s |
| 7 | [ClickHouse](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse) | 2.46s |
| 8 | [intent-gizmosql](https://github.com/ClickHouse/ClickBench/tree/main/intent-gizmosql) | 2.50s |
| 9 | [ClickHouse (web)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-web) | 2.66s |
| 10 | [chDB (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/chdb-dataframe) | 2.77s |
| 11 | [GizmoSQL](https://github.com/ClickHouse/ClickBench/tree/main/gizmosql) | 3.18s |
| 12 | [Umbra (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/umbra-parquet) | 3.24s |
| 13 | [Umbra (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/umbra-parquet-partitioned) | 3.52s |
| 14 | [DuckDB (memory)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-memory) | 3.72s |
| 15 | [pg_clickhouse](https://github.com/ClickHouse/ClickBench/tree/main/pg_clickhouse) | 4.00s |
| 16 | [Polars (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/polars-dataframe) | 4.73s |
| 17 | [StarRocks](https://github.com/ClickHouse/ClickBench/tree/main/starrocks) | 5.31s |
| 18 | [DuckDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet-partitioned) | 5.42s |
| 19 | [Polars (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/polars) | 5.91s |
| **20** | [**infino**](results/infino/c8g.metal-48xl.json) | **6.04s** |
| 21 | [DuckDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet) | 6.19s |
| 22 | [DuckDB (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake) | 6.42s |
| 23 | [DuckDB (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake-partitioned) | 6.52s |
| 24 | [Arc](https://github.com/ClickHouse/ClickBench/tree/main/arc) | 6.71s |
| 25 | [ClickHouse (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet-partitioned) | 6.90s |
| 26 | [DuckDB (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-dataframe) | 7.02s |
| 27 | [DataFusion (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-partitioned) | 7.09s |
| 28 | [chDB](https://github.com/ClickHouse/ClickBench/tree/main/chdb) | 7.19s |
| 29 | [chDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/chdb-parquet-partitioned) | 8.39s |
| 30 | [DataFusion (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion) | 8.50s |
| 31 | [ClickHouse (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake-partitioned) | 8.73s |
| 32 | [QuestDB](https://github.com/ClickHouse/ClickBench/tree/main/questdb) | 9.60s |
| 33 | [CedarDB (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/cedardb-parquet) | 9.79s |
| 34 | [DataFusion (Vortex, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex-partitioned) | 9.81s |
| 35 | [Sail (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/sail) | 9.89s |
| 36 | [ClickHouse (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet) | 10.11s |
| 37 | [Sail (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/sail-partitioned) | 10.61s |
| 38 | [DuckDB](https://github.com/ClickHouse/ClickBench/tree/main/duckdb) | 11.29s |
| 39 | [Spice.ai OSS (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet-partitioned) | 11.82s |
| 40 | [Spice.ai OSS (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet) | 11.89s |
| 41 | [ClickHouse (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake) | 11.94s |
| 42 | [Opteryx](https://github.com/ClickHouse/ClickBench/tree/main/opteryx-skene) | 17.28s |
| 43 | [VictoriaLogs](https://github.com/ClickHouse/ClickBench/tree/main/victorialogs) | 18.17s |
| 44 | [Firebolt (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet-partitioned) | 18.58s |
| 45 | [DuckDB (Vortex, single, load threads=2)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex-tuned) | 20.06s |
| 46 | [pg_duckdb (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb-parquet) | 21.70s |
| 47 | [StarRocks (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/starrocks-parquet-partitioned) | 23.20s |
| 48 | [StarRocks (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/starrocks-parquet) | 24.24s |
| 49 | [Opteryx (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/opteryx) | 29.07s |
| 50 | [DuckDB (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex) | 29.08s |
| 51 | [DataFusion (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex) | 29.71s |
| 52 | [Daft (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet) | 31.11s |
| 53 | [Velox (Axiom)](https://github.com/ClickHouse/ClickBench/tree/main/velox) | 31.97s |
| 54 | [Ravel](https://github.com/ClickHouse/ClickBench/tree/main/ravel) | 33.80s |
| 55 | [BemiDB](https://github.com/ClickHouse/ClickBench/tree/main/bemidb) | 35.78s |
| 56 | [Daft (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet-partitioned) | 43.05s |
| 57 | [GlareDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb-partitioned) | 44.22s |
| 58 | [GlareDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb) | 46.36s |
| 59 | [Spark (Comet)](https://github.com/ClickHouse/ClickBench/tree/main/spark-comet) | 61.04s |
| 60 | [Trino (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-partitioned) | 85.23s |
| 61 | [Trino (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino) | 90.61s |
| 62 | [Trino (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake-partitioned) | 97.42s |
| 63 | [Pinot](https://github.com/ClickHouse/ClickBench/tree/main/pinot) | 97.84s |
| 64 | [Trino (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake) | 105.07s |
| 65 | [SlateDB](https://github.com/ClickHouse/ClickBench/tree/main/slatedb) | 105.57s |
| 66 | [Pinot (with star-tree index)](https://github.com/ClickHouse/ClickBench/tree/main/pinot-tuned) | 107.27s |
| 67 | [Firebolt (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet) | 114.44s |
| 68 | [Presto (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/presto-partitioned) | 122.40s |
| 69 | [Cloudberry](https://github.com/ClickHouse/ClickBench/tree/main/cloudberry) | 127.80s |
| 70 | [WarehousePG](https://github.com/ClickHouse/ClickBench/tree/main/warehousepg) | 134.12s |
| 71 | [Presto (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/presto-datalake-partitioned) | 136.56s |
| 72 | [Presto (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/presto) | 152.19s |
| 73 | [Presto (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/presto-datalake) | 159.73s |
| 74 | [openGauss](https://github.com/ClickHouse/ClickBench/tree/main/opengauss) | 225.24s |
| 75 | [Spark](https://github.com/ClickHouse/ClickBench/tree/main/spark) | 300.89s |
| 76 | [TimescaleDB](https://github.com/ClickHouse/ClickBench/tree/main/timescaledb) | 495.27s |
| 77 | [CrateDB](https://github.com/ClickHouse/ClickBench/tree/main/cratedb) | 693.13s |
| 78 | [MariaDB ColumnStore](https://github.com/ClickHouse/ClickBench/tree/main/mariadb-columnstore) | 701.44s |
| 79 | [PostgreSQL](https://github.com/ClickHouse/ClickBench/tree/main/postgresql) | 715.83s |
| 80 | [Greenplum](https://github.com/ClickHouse/ClickBench/tree/main/greenplum) | 738.17s |
| 81 | [ParadeDB](https://github.com/ClickHouse/ClickBench/tree/main/paradedb) | 850.54s |
| 82 | [pandas](https://github.com/ClickHouse/ClickBench/tree/main/pandas) | 1418.23s |

Reference rows are best-per-system from upstream ClickBench's `c8g.metal-48xl` results; infino's row is its upstream submission. Managed warehouses (Snowflake, Databricks, BigQuery, Redshift) are excluded because they do not run on a fixed instance.

### How ClickBench measures

Each query is run three times. **Cold** is the first run (`t1`); **hot** is the best of the warm runs (`min(t2, t3)`). **Sum** is the total across all 43 queries; **geomean** is the geometric mean, so no single slow query dominates. Lower is better everywhere.

## Sources

All numbers are from upstream [ClickBench](https://github.com/ClickHouse/ClickBench). The rows mirrored here, with the upstream run date and the version measured:

- infino: [`infino/results/20260828/c6a.4xlarge.json`](https://github.com/ClickHouse/ClickBench/blob/main/infino/results/20260828/c6a.4xlarge.json) and [`infino/results/20260829/c8g.metal-48xl.json`](https://github.com/ClickHouse/ClickBench/blob/main/infino/results/20260829/c8g.metal-48xl.json) — infino **0.5.10**, taken 2026-08-28 (c6a) / 2026-08-29 (c8g).
- DataFusion (Parquet, single): [`datafusion/results/20260820/c6a.4xlarge.json`](https://github.com/ClickHouse/ClickBench/blob/main/datafusion/results/20260820/c6a.4xlarge.json)
- ClickHouse (Parquet, single): [`clickhouse-parquet/results/20261007/c6a.4xlarge.json`](https://github.com/ClickHouse/ClickBench/blob/main/clickhouse-parquet/results/20261007/c6a.4xlarge.json)
- ClickHouse (native): [`clickhouse/results/20261006/c6a.4xlarge.json`](https://github.com/ClickHouse/ClickBench/blob/main/clickhouse/results/20261006/c6a.4xlarge.json)

Managed cloud warehouses (Snowflake, Databricks, BigQuery, Redshift) are excluded because they do not run on c6a.4xlarge.

## Updating

Numbers are refreshed by re-reading the latest per-machine result files from upstream [ClickBench](https://github.com/ClickHouse/ClickBench): the newest `results/<date>/<machine>.json` for infino and each reference engine, taking the best hot sum per system for the ranks.
