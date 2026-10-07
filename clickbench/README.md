# ClickBench results

infino's ClickBench numbers against the published reference engines, on the ClickBench reference machine **c6a.4xlarge** at the full 100M-row scale.

Across the full self-hosted ClickBench field on c6a.4xlarge, infino ranks **#35 of 126** engines by hot-run total. See the [full comparison](FULL_COMPARISON.md) for all 126.

This README keeps the headline comparison against the two engines that matter most for us, DataFusion and ClickHouse. All numbers are from upstream [ClickBench](https://github.com/ClickHouse/ClickBench), where infino now has published results; each row links to its folder there, and the result files are mirrored under `results/`. Ranks use the **newest complete run per system**: each system's most recent upstream run with all 43 queries (hot = min of tries 2 and 3), applied to infino's row too. Snapshot: upstream [ClickBench](https://github.com/ClickHouse/ClickBench) at `6152b6b`.

## Results (c6a.4xlarge, 100M rows)

| System | Cold sum | Cold geomean | Hot sum | Hot geomean |
|---|--:|--:|--:|--:|
| [**infino**](https://github.com/ClickHouse/ClickBench/tree/main/infino) ([file](results/infino/c6a.4xlarge.json)) | 128.46s | 1.891s | **33.74s** | **0.2636s** |
| [DataFusion (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion) ([file](results/datafusion/c6a.4xlarge.json)) | 182.91s | 1.169s | 45.57s | 0.3558s |
| [ClickHouse (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet) ([file](results/clickhouse-parquet/c6a.4xlarge.json)) | 127.72s | 1.247s | 28.39s | 0.3218s |
| [ClickHouse (native, MergeTree)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse) ([file](results/clickhouse/c6a.4xlarge.json)) | 106.08s | 1.004s | 17.44s | 0.1039s |

On hot, infino beats DataFusion on both total (33.74s vs 45.57s) and geomean (0.2636 vs 0.3558). Against ClickHouse-on-Parquet it trails on total (33.74s vs 28.39s) but leads on geomean (0.2636 vs 0.3218) — one slow query weighs on infino's total while its per-query distribution is tighter. ClickHouse's native MergeTree is faster on both, on a different substrate (it ingests into its own format rather than reading Parquet).

On cold, infino's total is competitive — close to ClickHouse-on-Parquet (128.46s vs 127.72s) and well ahead of DataFusion (182.91s) — but its cold geomean is the highest of the four (1.891s against 1.004s-1.247s): a few slow cold queries weigh on the per-query figure even though the total holds up.

## The leaderboard machine (c8g.metal-48xl, 100M rows)

The numbers quoted off the ClickBench homepage come from the largest machine, **c8g.metal-48xl** (Graviton4, 192 vCPU), not the c6a.4xlarge reference above. infino's result there: **hot sum 6.04s, geomean 0.0837** ([result file](results/infino/c8g.metal-48xl.json)).

Full field on this machine, best hot sum per system (86 systems). infino ranks **#19**, ahead of every DataFusion build and every ClickHouse-on-Parquet variant. Ahead of it sit the native-format and in-memory engines, plus a handful of newer Parquet/dataframe readers.

| # | System | Hot sum |
|--:|---|--:|
| 1 | [elosdb](https://github.com/ClickHouse/ClickBench/tree/main/elosdb) | 0.87s |
| 2 | [Pivotlake (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/pivot-parquet-partitioned) | 1.03s |
| 3 | [intent-gizmosql](https://github.com/ClickHouse/ClickBench/tree/main/intent-gizmosql) | 1.04s |
| 4 | [Pivotlake (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/pivot-parquet) | 1.05s |
| 5 | [Umbra](https://github.com/ClickHouse/ClickBench/tree/main/umbra) | 1.17s |
| 6 | [CedarDB](https://github.com/ClickHouse/ClickBench/tree/main/cedardb) | 2.38s |
| 7 | [Firebolt](https://github.com/ClickHouse/ClickBench/tree/main/firebolt) | 2.66s |
| 8 | [Rayforce](https://github.com/ClickHouse/ClickBench/tree/main/rayforce) | 2.67s |
| 9 | [ClickHouse](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse) | 2.67s |
| 10 | [ClickHouse (web)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-web) | 2.72s |
| 11 | [chDB (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/chdb-dataframe) | 2.77s |
| 12 | [GizmoSQL](https://github.com/ClickHouse/ClickBench/tree/main/gizmosql) | 3.24s |
| 13 | [Umbra (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/umbra-parquet) | 3.24s |
| 14 | [Umbra (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/umbra-parquet-partitioned) | 3.52s |
| 15 | [DuckDB (memory)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-memory) | 3.72s |
| 16 | [pg_clickhouse](https://github.com/ClickHouse/ClickBench/tree/main/pg_clickhouse) | 4.49s |
| 17 | [Polars (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/polars-dataframe) | 4.96s |
| 18 | [StarRocks](https://github.com/ClickHouse/ClickBench/tree/main/starrocks) | 5.84s |
| **19** | [**infino**](results/infino/c8g.metal-48xl.json) | **6.04s** |
| 20 | [Arc](https://github.com/ClickHouse/ClickBench/tree/main/arc) | 6.79s |
| 21 | [ClickHouse (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet-partitioned) | 6.93s |
| 22 | [Polars (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/polars) | 7.00s |
| 23 | [DataFusion (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-partitioned) | 7.09s |
| 24 | [chDB](https://github.com/ClickHouse/ClickBench/tree/main/chdb) | 7.19s |
| 25 | [chDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/chdb-parquet-partitioned) | 8.39s |
| 26 | [DataFusion (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion) | 8.50s |
| 27 | [QuestDB](https://github.com/ClickHouse/ClickBench/tree/main/questdb) | 9.60s |
| 28 | [DataFusion (Vortex, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex-partitioned) | 9.81s |
| 29 | [Sail (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/sail) | 9.89s |
| 30 | [CedarDB (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/cedardb-parquet) | 10.12s |
| 31 | [ScramDB](https://github.com/ClickHouse/ClickBench/tree/main/scramdb) | 10.55s |
| 32 | [Sail (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/sail-partitioned) | 10.61s |
| 33 | [ClickHouse (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet) | 10.85s |
| 34 | [ClickHouse (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake-partitioned) | 11.24s |
| 35 | [DuckDB](https://github.com/ClickHouse/ClickBench/tree/main/duckdb) | 11.29s |
| 36 | [DuckDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet-partitioned) | 11.47s |
| 37 | [Spice.ai OSS (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet-partitioned) | 11.82s |
| 38 | [Spice.ai OSS (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet) | 11.89s |
| 39 | [DuckDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet) | 12.22s |
| 40 | [ClickHouse (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake) | 13.39s |
| 41 | [Opteryx](https://github.com/ClickHouse/ClickBench/tree/main/opteryx-skene) | 17.28s |
| 42 | [VictoriaLogs](https://github.com/ClickHouse/ClickBench/tree/main/victorialogs) | 18.17s |
| 43 | [DuckDB (Vortex, single, load threads=2)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex-tuned) | 20.06s |
| 44 | [Firebolt (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet-partitioned) | 20.66s |
| 45 | [pg_duckdb (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb-parquet) | 21.70s |
| 46 | [StarRocks (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/starrocks-parquet-partitioned) | 23.64s |
| 47 | [StarRocks (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/starrocks-parquet) | 24.70s |
| 48 | [Opteryx (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/opteryx) | 29.07s |
| 49 | [DataFusion (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex) | 29.71s |
| 50 | [Velox (Axiom)](https://github.com/ClickHouse/ClickBench/tree/main/velox) | 31.97s |
| 51 | [Ravel](https://github.com/ClickHouse/ClickBench/tree/main/ravel) | 34.62s |
| 52 | [BemiDB](https://github.com/ClickHouse/ClickBench/tree/main/bemidb) | 35.78s |
| 53 | [DuckDB (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex) | 37.40s |
| 54 | [Daft (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet-partitioned) | 43.05s |
| 55 | [DuckDB (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake) | 47.06s |
| 56 | [GlareDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb-partitioned) | 47.42s |
| 57 | [GlareDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb) | 49.03s |
| 58 | [DuckDB (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake-partitioned) | 51.56s |
| 59 | [GenDB](https://github.com/ClickHouse/ClickBench/tree/main/gendb) | 70.06s |
| 60 | [Daft (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet) | 87.38s |
| 61 | [Trino (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-partitioned) | 96.19s |
| 62 | [Trino (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino) | 100.17s |
| 63 | [Pinot](https://github.com/ClickHouse/ClickBench/tree/main/pinot) | 101.29s |
| 64 | [SlateDB](https://github.com/ClickHouse/ClickBench/tree/main/slatedb) | 105.57s |
| 65 | [Pinot (with star-tree index)](https://github.com/ClickHouse/ClickBench/tree/main/pinot-tuned) | 107.27s |
| 66 | [Cloudberry](https://github.com/ClickHouse/ClickBench/tree/main/cloudberry) | 127.80s |
| 67 | [Presto (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/presto-partitioned) | 131.02s |
| 68 | [WarehousePG](https://github.com/ClickHouse/ClickBench/tree/main/warehousepg) | 134.12s |
| 69 | [Presto (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/presto-datalake-partitioned) | 143.47s |
| 70 | [Presto (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/presto) | 156.94s |
| 71 | [Trino (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake-partitioned) | 159.64s |
| 72 | [Presto (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/presto-datalake) | 159.73s |
| 73 | [Trino (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake) | 179.43s |
| 74 | [Firebolt (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet) | 208.34s |
| 75 | [openGauss](https://github.com/ClickHouse/ClickBench/tree/main/opengauss) | 225.24s |
| 76 | [Spark (Comet)](https://github.com/ClickHouse/ClickBench/tree/main/spark-comet) | 261.01s |
| 77 | [Spark](https://github.com/ClickHouse/ClickBench/tree/main/spark) | 300.89s |
| 78 | [TimescaleDB](https://github.com/ClickHouse/ClickBench/tree/main/timescaledb) | 495.27s |
| 79 | [CrateDB](https://github.com/ClickHouse/ClickBench/tree/main/cratedb) | 693.13s |
| 80 | [MariaDB ColumnStore](https://github.com/ClickHouse/ClickBench/tree/main/mariadb-columnstore) | 701.44s |
| 81 | [PostgreSQL](https://github.com/ClickHouse/ClickBench/tree/main/postgresql) | 715.83s |
| 82 | [Greenplum](https://github.com/ClickHouse/ClickBench/tree/main/greenplum) | 743.99s |
| 83 | [ParadeDB](https://github.com/ClickHouse/ClickBench/tree/main/paradedb) | 850.54s |
| 84 | [pandas](https://github.com/ClickHouse/ClickBench/tree/main/pandas) | 1418.23s |
| 85 | [BQN](https://github.com/ClickHouse/ClickBench/tree/main/bqn) | 1447.20s |
| 86 | [DuckDB (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-dataframe) | 5005.50s |

Managed warehouses (Snowflake, Databricks, BigQuery, Redshift) are excluded because they do not run on a fixed instance.

### How ClickBench measures

Each query is run three times. **Cold** is the first run (`t1`); **hot** is the best of the warm runs (`min(t2, t3)`). **Sum** is the total across all 43 queries; **geomean** is the geometric mean, so no single slow query dominates. Lower is better everywhere.

## Sources

All numbers are from upstream [ClickBench](https://github.com/ClickHouse/ClickBench), snapshot `6152b6b`. The rows mirrored here, with the upstream run date and the version measured:

- infino: [`infino/results/20260828/c6a.4xlarge.json`](https://github.com/ClickHouse/ClickBench/blob/main/infino/results/20260828/c6a.4xlarge.json) and [`infino/results/20260829/c8g.metal-48xl.json`](https://github.com/ClickHouse/ClickBench/blob/main/infino/results/20260829/c8g.metal-48xl.json) — infino **0.5.10**, taken 2026-08-28 (c6a) / 2026-08-29 (c8g).
- DataFusion (Parquet, single): [`datafusion/results/20260820/c6a.4xlarge.json`](https://github.com/ClickHouse/ClickBench/blob/main/datafusion/results/20260820/c6a.4xlarge.json)
- ClickHouse (Parquet, single): [`clickhouse-parquet/results/20261007/c6a.4xlarge.json`](https://github.com/ClickHouse/ClickBench/blob/main/clickhouse-parquet/results/20261007/c6a.4xlarge.json)
- ClickHouse (native): [`clickhouse/results/20261007/c6a.4xlarge.json`](https://github.com/ClickHouse/ClickBench/blob/main/clickhouse/results/20261007/c6a.4xlarge.json)

Managed cloud warehouses (Snowflake, Databricks, BigQuery, Redshift) are excluded because they do not run on c6a.4xlarge.

## Updating

Numbers are refreshed by re-reading upstream [ClickBench](https://github.com/ClickHouse/ClickBench) at a pinned commit: for each system the newest `results/<date>/<machine>.json` with all 43 queries, hot = min(t2, t3), ranked by hot sum.
