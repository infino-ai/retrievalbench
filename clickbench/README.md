# ClickBench results

infino is on the public [ClickBench](https://benchmark.clickhouse.com/) board, measured by ClickBench's own harness and machines from the [`infino/`](https://github.com/ClickHouse/ClickBench/tree/main/infino) folder upstream, at the full 100M-row scale.

## Where infino stands on the board

Positions as the board itself scores them, in its default view (CPU, untuned, and the board's default machine set, which leaves out c6a.large, c6a.xlarge, c6a.2xlarge, c6a.metal, c8g.4xlarge and t3a.small), read from [benchmark.clickhouse.com](https://benchmark.clickhouse.com/) on 2026-10-20 for infino **0.11.3**, when the board was built from upstream ClickBench `57db2a0` (open its `index.html` at that commit to reproduce every position here). The board lists every machine a system runs on as its own entry; each row below is infino's best entry.

| Metric | infino's best entry | Position on the board |
|---|---|--:|
| [Combined](https://benchmark.clickhouse.com/#system=-&type=-&machine=-6t%7Cca2%7C6ax%7Cg4e%7C6ale%7C3al&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=combined&queries=-) (hot + cold + load + size) | c8g.metal-48xl | **#77 of 478** (top 16%) |
| [Hot run](https://benchmark.clickhouse.com/#system=-&type=-&machine=-6t%7Cca2%7C6ax%7Cg4e%7C6ale%7C3al&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=hot&queries=-) | c8g.metal-48xl | **#55 of 499** (top 11%) |
| [Storage size](https://benchmark.clickhouse.com/#system=-&type=-&machine=-6t%7Cca2%7C6ax%7Cg4e%7C6ale%7C3al&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=size&queries=-) | c8g.metal-48xl, 9.27 GiB | **#32 of 498** (top 6%) |
| [Load time](https://benchmark.clickhouse.com/#system=-&type=-&machine=-6t%7Cca2%7C6ax%7Cg4e%7C6ale%7C3al&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=load&queries=-) | c8g.metal-48xl, 89 s | **#82 of 303** (top 27%) |
| [Cold run](https://benchmark.clickhouse.com/#system=-&type=-&machine=-6t%7Cca2%7C6ax%7Cg4e%7C6ale%7C3al&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=cold&queries=-) | c6a.4xlarge | #146 of 309 (top 47%) |

Cold is infino's weakest metric today: most of the change since 0.5.10 is a single query (Q22), which is being worked on. Among search engines infino is first on cold too (below).

The two comparisons below use the board with **all machines** selected, and each cell is that system's best entry for that metric on any machine, so cells in one row can come from different machines (the links open each view).

- **#1 among search engines.** Of the systems the board tags `search` (Elasticsearch, Quickwit, ParadeDB, SigLens, infino), infino is first on every metric: [combined](https://benchmark.clickhouse.com/#system=-&type=+sac&machine=-&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=combined&queries=-), [hot](https://benchmark.clickhouse.com/#system=-&type=+sac&machine=-&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=hot&queries=-), [cold](https://benchmark.clickhouse.com/#system=-&type=+sac&machine=-&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=cold&queries=-), [load](https://benchmark.clickhouse.com/#system=-&type=+sac&machine=-&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=load&queries=-) and [size](https://benchmark.clickhouse.com/#system=-&type=+sac&machine=-&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=size&queries=-), and all eight of its 0.11.3 runs, one per machine, come before the next search engine on combined and hot. The best entry of each of the others, relative to infino's best:

  | System | Combined | Hot | Cold | Load | Size |
  |---|--:|--:|--:|--:|--:|
  | **infino** | **×1.0** | **×1.0** | **×1.0** | **89 s** | **9.27 GiB** |
  | Quickwit | ×12.5 | ×19.3 | ×3.8 | 2339 s | 43.13 GiB |
  | Elasticsearch | ×20.8 | ×24.5 | no true cold run | 9531 s | 78.85 GiB |
  | SigLens | ×58.1 | ×167.6 | ×13.6 | 5016 s | 26.92 GiB |
  | ParadeDB | ×61.8 | ×82.3 | ×172.1 | 1611 s | 75.15 GiB |

  Sizes are infino's 0.11.3 runs only: the board still lists one older 0.5.10 entry for infino (t3a.small, 9.12 GiB), which is not used here.

- **Ahead of every Snowflake and Databricks configuration** on [combined](https://benchmark.clickhouse.com/#system=+abk%7CIfn%7Cnfe%7Cw%20en%7CSIt&type=-&machine=-&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=combined&queries=-) and [hot](https://benchmark.clickhouse.com/#system=+abk%7CIfn%7Cnfe%7Cw%20en%7CSIt&type=-&machine=-&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=hot&queries=-), from a single machine against warehouse clusters of up to 256 nodes: on combined infino's best entry scores ×1.29, the best Snowflake (Interactive, 32×2XL) ×1.93 and the best Databricks (32×XL) ×4.75 (lower is better). infino also stores the dataset smaller than any of them ([size](https://benchmark.clickhouse.com/#system=+abk%7CIfn%7Cnfe%7Cw%20en%7CSIt&type=-&machine=-&cluster_size=-&opensource=-&hardware=+c&tuned=+n&metric=size&queries=-)): 9.27 GiB against Databricks' smallest at 9.52 GiB. On load the large warehouses are faster: Databricks 30 s on 256×4XL, Snowflake 67 s on 32×2XL, infino 89 s on one box. They have no true cold run, so the board leaves them out of cold.
- **Load is ~10x faster than in 0.5.10** (c8g.metal-48xl 1061 s -> 89 s, c6a.4xlarge 1139 s -> 130 s): the table is now built in one bulk pass instead of ingest followed by optimize. On c6a.4xlarge infino now loads faster than ClickHouse's native MergeTree (130 s vs 182 s) and stores the 13.76 GiB of input Parquet in 9.33 GiB.

## Head-to-head on hot-run total

The tables below rank by a simpler rule than the board's: the hot-run total across the 43 queries. Across the full self-hosted ClickBench field on c6a.4xlarge, infino ranks **#34 of 129** engines by hot-run total. See the [full comparison](FULL_COMPARISON.md) for all 129.

This section keeps the headline comparison against the two engines that matter most for us, DataFusion and ClickHouse. All numbers are from upstream [ClickBench](https://github.com/ClickHouse/ClickBench), where infino has published results; each row links to its folder there and to its result file at that snapshot. Ranks use the **newest complete run per system**: each system's most recent upstream run with all 43 queries (hot = min of tries 2 and 3), applied to infino's row too. Snapshot: upstream [ClickBench](https://github.com/ClickHouse/ClickBench) at `57db2a0`.

### c6a.4xlarge (the reference machine, 100M rows)

| System | Cold sum | Cold geomean | Hot sum | Hot geomean | Load | Size |
|---|--:|--:|--:|--:|--:|--:|
| [**infino**](https://github.com/ClickHouse/ClickBench/tree/main/infino) ([file](https://github.com/ClickHouse/ClickBench/blob/57db2a04749263e7eaa4c2f8806bae4c16d7b8ff/infino/results/20261014/c6a.4xlarge.json)) | 133.04s | 1.794s | **30.52s** | **0.2081s** | 130s | 9.33 GiB |
| [DataFusion (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion) ([file](https://github.com/ClickHouse/ClickBench/blob/57db2a04749263e7eaa4c2f8806bae4c16d7b8ff/datafusion/results/20260820/c6a.4xlarge.json)) | 182.91s | 1.169s | 45.57s | 0.3558s | 10s* | 13.76 GiB |
| [ClickHouse (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet) ([file](https://github.com/ClickHouse/ClickBench/blob/57db2a04749263e7eaa4c2f8806bae4c16d7b8ff/clickhouse-parquet/results/20261020/c6a.4xlarge.json)) | 128.53s | 1.294s | 27.11s | 0.3105s | 0s* | 13.76 GiB |
| [ClickHouse (native, MergeTree)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse) ([file](https://github.com/ClickHouse/ClickBench/blob/57db2a04749263e7eaa4c2f8806bae4c16d7b8ff/clickhouse/results/20261020/c6a.4xlarge.json)) | 95.74s | 0.924s | 16.03s | 0.0933s | 182s | 8.77 GiB |

\* These two query the benchmark's Parquet file in place instead of loading it into a format of their own: their load time is only the setup their load script does, and their size is the Parquet file itself.

On hot, infino beats DataFusion on both total (30.52s vs 45.57s) and geomean (0.2081 vs 0.3558). Against ClickHouse-on-Parquet it trails on total (30.52s vs 27.11s) but leads on geomean (0.2081 vs 0.3105); one slow query weighs on infino's total while its per-query distribution is tighter. ClickHouse's native MergeTree is faster on both, on a different substrate (it ingests into its own format rather than reading Parquet).

On cold, infino's total is competitive, close to ClickHouse-on-Parquet (133.04s vs 128.53s) and well ahead of DataFusion (182.91s), but its cold geomean is the highest of the four (1.794s against 0.924s-1.294s): a few slow cold queries weigh on the per-query figure even though the total holds up.

### c8g.metal-48xl (the leaderboard machine, 100M rows)

The numbers quoted off the ClickBench homepage come from the largest machine, **c8g.metal-48xl** (Graviton4, 192 vCPU), not the c6a.4xlarge reference above. infino's result there: **hot sum 6.08s, geomean 0.0754** ([result file](https://github.com/ClickHouse/ClickBench/blob/57db2a04749263e7eaa4c2f8806bae4c16d7b8ff/infino/results/20261014/c8g.metal-48xl.json)).

Full field on this machine, newest complete run per system (88 systems). infino ranks **#23**, ahead of every DataFusion build on Parquet and every ClickHouse-on-Parquet variant; only DataFusion on Vortex (partitioned) is just ahead, at 5.94s. Ahead of it sit the native-format and in-memory engines, plus a handful of newer Parquet/dataframe readers.

| # | System | Hot sum |
|--:|---|--:|
| 1 | [intent-gizmosql](https://github.com/ClickHouse/ClickBench/tree/main/intent-gizmosql) | 0.52s |
| 2 | [Umbra](https://github.com/ClickHouse/ClickBench/tree/main/umbra) | 0.73s |
| 3 | [elosdb](https://github.com/ClickHouse/ClickBench/tree/main/elosdb) | 0.87s |
| 4 | [Pivotlake (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/pivot-parquet-partitioned) | 1.03s |
| 5 | [Pivotlake (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/pivot-parquet) | 1.05s |
| 6 | [CedarDB](https://github.com/ClickHouse/ClickBench/tree/main/cedardb) | 2.38s |
| 7 | [Rayforce](https://github.com/ClickHouse/ClickBench/tree/main/rayforce) | 2.50s |
| 8 | [ClickHouse (web)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-web) | 2.61s |
| 9 | [Umbra (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/umbra-parquet) | 2.62s |
| 10 | [Firebolt](https://github.com/ClickHouse/ClickBench/tree/main/firebolt) | 2.66s |
| 11 | [ClickHouse](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse) | 2.69s |
| 12 | [chDB (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/chdb-dataframe) | 2.77s |
| 13 | [DuckFlight (Flight SQL)](https://github.com/ClickHouse/ClickBench/tree/main/duckflight) | 2.92s |
| 14 | [Umbra (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/umbra-parquet-partitioned) | 2.94s |
| 15 | [GizmoSQL](https://github.com/ClickHouse/ClickBench/tree/main/gizmosql) | 3.24s |
| 16 | [DuckDB (memory)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-memory) | 3.72s |
| 17 | [chDB](https://github.com/ClickHouse/ClickBench/tree/main/chdb) | 4.44s |
| 18 | [pg_clickhouse](https://github.com/ClickHouse/ClickBench/tree/main/pg_clickhouse) | 4.49s |
| 19 | [Polars (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/polars-dataframe) | 4.96s |
| 20 | [Opteryx (`opteryx-parquet`)](https://github.com/ClickHouse/ClickBench/tree/main/opteryx-parquet) | 5.48s |
| 21 | [StarRocks](https://github.com/ClickHouse/ClickBench/tree/main/starrocks) | 5.84s |
| 22 | [DataFusion (Vortex, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex-partitioned) | 5.94s |
| **23** | [**infino**](results/infino/c8g.metal-48xl.json) | **6.08s** |
| 24 | [chDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/chdb-parquet-partitioned) | 6.49s |
| 25 | [Polars (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/polars) | 6.56s |
| 26 | [ClickHouse (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet-partitioned) | 6.73s |
| 27 | [Arc](https://github.com/ClickHouse/ClickBench/tree/main/arc) | 6.79s |
| 28 | [DataFusion (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-partitioned) | 7.09s |
| 29 | [Opteryx (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/opteryx) | 7.51s |
| 30 | [DataFusion (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex) | 8.36s |
| 31 | [DataFusion (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion) | 8.50s |
| 32 | [QuestDB](https://github.com/ClickHouse/ClickBench/tree/main/questdb) | 9.60s |
| 33 | [Sail (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/sail) | 9.86s |
| 34 | [CedarDB (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/cedardb-parquet) | 10.12s |
| 35 | [Sail (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/sail-partitioned) | 10.37s |
| 36 | [ScramDB](https://github.com/ClickHouse/ClickBench/tree/main/scramdb) | 10.55s |
| 37 | [ClickHouse (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake-partitioned) | 10.86s |
| 38 | [ClickHouse (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet) | 11.18s |
| 39 | [DuckDB](https://github.com/ClickHouse/ClickBench/tree/main/duckdb) | 11.29s |
| 40 | [DuckDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet-partitioned) | 11.47s |
| 41 | [Spice.ai OSS (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet-partitioned) | 11.82s |
| 42 | [Spice.ai OSS (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet) | 11.89s |
| 43 | [DuckDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet) | 12.22s |
| 44 | [ClickHouse (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake) | 13.05s |
| 45 | [Ravel](https://github.com/ClickHouse/ClickBench/tree/main/ravel) | 15.39s |
| 46 | [Opteryx (`opteryx-skene`)](https://github.com/ClickHouse/ClickBench/tree/main/opteryx-skene) | 17.28s |
| 47 | [VictoriaLogs](https://github.com/ClickHouse/ClickBench/tree/main/victorialogs) | 18.17s |
| 48 | [DuckDB (Vortex, single, load threads=2)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex-tuned) | 20.06s |
| 49 | [Firebolt (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet-partitioned) | 20.66s |
| 50 | [pg_duckdb (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb-parquet) | 21.70s |
| 51 | [StarRocks (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/starrocks-parquet-partitioned) | 23.64s |
| 52 | [StarRocks (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/starrocks-parquet) | 24.70s |
| 53 | [Velox (Axiom)](https://github.com/ClickHouse/ClickBench/tree/main/velox) | 31.97s |
| 54 | [BemiDB](https://github.com/ClickHouse/ClickBench/tree/main/bemidb) | 35.78s |
| 55 | [DuckDB (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex) | 37.40s |
| 56 | [Daft (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet-partitioned) | 43.05s |
| 57 | [DuckDB (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake) | 47.06s |
| 58 | [GlareDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb-partitioned) | 47.42s |
| 59 | [GlareDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb) | 49.03s |
| 60 | [DuckDB (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake-partitioned) | 51.56s |
| 61 | [GenDB](https://github.com/ClickHouse/ClickBench/tree/main/gendb) | 70.06s |
| 62 | [Daft (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet) | 87.38s |
| 63 | [Trino (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-partitioned) | 96.19s |
| 64 | [Trino (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino) | 100.17s |
| 65 | [Pinot](https://github.com/ClickHouse/ClickBench/tree/main/pinot) | 101.29s |
| 66 | [SlateDB](https://github.com/ClickHouse/ClickBench/tree/main/slatedb) | 105.57s |
| 67 | [Pinot (with star-tree index)](https://github.com/ClickHouse/ClickBench/tree/main/pinot-tuned) | 107.27s |
| 68 | [Presto (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/presto-partitioned) | 122.40s |
| 69 | [Cloudberry](https://github.com/ClickHouse/ClickBench/tree/main/cloudberry) | 127.80s |
| 70 | [WarehousePG](https://github.com/ClickHouse/ClickBench/tree/main/warehousepg) | 134.12s |
| 71 | [Presto (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/presto-datalake-partitioned) | 143.47s |
| 72 | [Presto (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/presto) | 154.31s |
| 73 | [Trino (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake-partitioned) | 159.64s |
| 74 | [Presto (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/presto-datalake) | 159.73s |
| 75 | [Trino (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake) | 179.43s |
| 76 | [Firebolt (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet) | 208.34s |
| 77 | [openGauss](https://github.com/ClickHouse/ClickBench/tree/main/opengauss) | 225.24s |
| 78 | [Spark (Comet)](https://github.com/ClickHouse/ClickBench/tree/main/spark-comet) | 261.01s |
| 79 | [Spark](https://github.com/ClickHouse/ClickBench/tree/main/spark) | 300.89s |
| 80 | [TimescaleDB](https://github.com/ClickHouse/ClickBench/tree/main/timescaledb) | 495.27s |
| 81 | [CrateDB](https://github.com/ClickHouse/ClickBench/tree/main/cratedb) | 693.13s |
| 82 | [MariaDB ColumnStore](https://github.com/ClickHouse/ClickBench/tree/main/mariadb-columnstore) | 701.44s |
| 83 | [PostgreSQL](https://github.com/ClickHouse/ClickBench/tree/main/postgresql) | 715.83s |
| 84 | [Greenplum](https://github.com/ClickHouse/ClickBench/tree/main/greenplum) | 743.99s |
| 85 | [ParadeDB](https://github.com/ClickHouse/ClickBench/tree/main/paradedb) | 850.54s |
| 86 | [pandas](https://github.com/ClickHouse/ClickBench/tree/main/pandas) | 1337.15s |
| 87 | [BQN](https://github.com/ClickHouse/ClickBench/tree/main/bqn) | 1447.20s |
| 88 | [DuckDB (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-dataframe) | 5005.50s |

Managed warehouses (Snowflake, Databricks, BigQuery, Redshift) are excluded because they do not run on a fixed instance.

infino's runs on the other board machines (c7a.metal-48xl, c6a.metal, c8g.4xlarge, c6a.2xlarge, c6a.xlarge, c6a.large) from the same release are in [`infino/results/20261014/`](https://github.com/ClickHouse/ClickBench/tree/57db2a04749263e7eaa4c2f8806bae4c16d7b8ff/infino/results/20261014).

### How ClickBench measures

Each query is run three times. **Cold** is the first run (`t1`); **hot** is the best of the warm runs (`min(t2, t3)`). **Sum** is the total across all 43 queries; **geomean** is the geometric mean, so no single slow query dominates. Lower is better everywhere.

The board's own positions (the first section) are relative rather than totals: for each query, an entry's time is divided by the fastest entry's time, and the ratios are combined with a geometric mean. **Combined** weights hot 60%, cold 20%, load 10% and size 10%.

## Sources

Board positions were read from [benchmark.clickhouse.com](https://benchmark.clickhouse.com/) on 2026-10-20. All other numbers are from upstream [ClickBench](https://github.com/ClickHouse/ClickBench), snapshot `57db2a0`. The result files behind each row, at that snapshot, with the upstream run date and the version measured:

- infino: [`infino/results/20261014/c6a.4xlarge.json`](https://github.com/ClickHouse/ClickBench/blob/57db2a04749263e7eaa4c2f8806bae4c16d7b8ff/infino/results/20261014/c6a.4xlarge.json) and [`infino/results/20261014/c8g.metal-48xl.json`](https://github.com/ClickHouse/ClickBench/blob/57db2a04749263e7eaa4c2f8806bae4c16d7b8ff/infino/results/20261014/c8g.metal-48xl.json): infino **0.11.3**, taken 2026-10-14 on both machines.
- DataFusion (Parquet, single): [`datafusion/results/20260820/c6a.4xlarge.json`](https://github.com/ClickHouse/ClickBench/blob/57db2a04749263e7eaa4c2f8806bae4c16d7b8ff/datafusion/results/20260820/c6a.4xlarge.json)
- ClickHouse (Parquet, single): [`clickhouse-parquet/results/20261020/c6a.4xlarge.json`](https://github.com/ClickHouse/ClickBench/blob/57db2a04749263e7eaa4c2f8806bae4c16d7b8ff/clickhouse-parquet/results/20261020/c6a.4xlarge.json)
- ClickHouse (native): [`clickhouse/results/20261020/c6a.4xlarge.json`](https://github.com/ClickHouse/ClickBench/blob/57db2a04749263e7eaa4c2f8806bae4c16d7b8ff/clickhouse/results/20261020/c6a.4xlarge.json)

Managed cloud warehouses (Snowflake, Databricks, BigQuery, Redshift) are excluded because they do not run on c6a.4xlarge.

## Updating

Numbers are refreshed by re-reading upstream [ClickBench](https://github.com/ClickHouse/ClickBench) at a pinned commit: for each system the newest `results/<date>/<machine>.json` with all 43 queries, hot = min(t2, t3), ranked by hot sum (geomean floors each query at 1 ms). Result files are linked at that commit rather than copied here, and the board positions are re-read from the board built at the same commit.
