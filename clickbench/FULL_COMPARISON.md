# ClickBench full comparison (c6a.4xlarge, 100M rows)

Every self-hosted engine on the ClickBench board with a complete c6a.4xlarge run, ranked by hot-run total across the 43 queries. infino sits at **#22 of 94**.

Reference numbers are sourced from upstream [ClickBench](https://github.com/ClickHouse/ClickBench); each system links to its folder there. Managed cloud warehouses (Snowflake, Databricks, BigQuery, Redshift, and similar) are excluded because they do not run on c6a.4xlarge. Only infino's number is measured by us; see the [README](README.md) for methodology and the correctness check.

| # | System | Hot sum | Hot geomean |
|--:|---|--:|--:|
| 1 | [GizmoSQL](https://github.com/ClickHouse/ClickBench/tree/main/gizmosql) | 21.03s | 0.1042 |
| 2 | [Firebolt](https://github.com/ClickHouse/ClickBench/tree/main/firebolt) | 21.25s | 0.1945 |
| 3 | [DuckDB (memory)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-memory) | 22.08s | 0.1186 |
| 4 | [MariaDB (DuckDB)](https://github.com/ClickHouse/ClickBench/tree/main/mariadb-duckdb) | 22.91s | 0.1092 |
| 5 | [ClickHouse (TCHouse-C)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-tencent) | 24.48s | 0.1139 |
| 6 | [Arc](https://github.com/ClickHouse/ClickBench/tree/main/arc) | 24.90s | 0.2114 |
| 7 | [Ursa](https://github.com/ClickHouse/ClickBench/tree/main/ursa) | 26.15s | 0.1248 |
| 8 | [Salesforce Hyper](https://github.com/ClickHouse/ClickBench/tree/main/hyper) | 26.25s | 0.1009 |
| 9 | [DuckDB](https://github.com/ClickHouse/ClickBench/tree/main/duckdb) | 26.25s | 0.2287 |
| 10 | [pg_deltax](https://github.com/ClickHouse/ClickBench/tree/main/pg_deltax) | 26.88s | 0.2419 |
| 11 | [DuckDB (Vortex, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex-partitioned) | 26.91s | 0.1489 |
| 12 | [QuestDB](https://github.com/ClickHouse/ClickBench/tree/main/questdb) | 26.96s | 0.1085 |
| 13 | [CedarDB (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/cedardb-parquet) | 29.37s | 0.3641 |
| 14 | [DuckDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet-partitioned) | 30.95s | 0.2662 |
| 15 | [ClickHouse](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse) | 32.26s | 0.1306 |
| 16 | [DuckDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet) | 32.48s | 0.3446 |
| 17 | [pg_clickhouse](https://github.com/ClickHouse/ClickBench/tree/main/pg_clickhouse) | 33.60s | 0.1548 |
| 18 | [chDB](https://github.com/ClickHouse/ClickBench/tree/main/chdb) | 33.82s | 0.2001 |
| 19 | [Polars (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/polars-dataframe) | 34.12s | 0.2344 |
| 20 | [Spice.ai OSS (Cayenne)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-cayenne) | 34.46s | 0.2447 |
| 21 | [CedarDB](https://github.com/ClickHouse/ClickBench/tree/main/cedardb) | 37.14s | 0.0693 |
| 22 | [**infino**](results/infino/c6a.4xlarge.json) | **37.37s** | **0.3100** |
| 23 | [Polars (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/polars) | 37.62s | 0.2857 |
| 24 | [DuckDB (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex) | 40.99s | 0.3269 |
| 25 | [Spice.ai OSS (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet-partitioned) | 41.41s | 0.3807 |
| 26 | [pg_ducklake](https://github.com/ClickHouse/ClickBench/tree/main/pg_ducklake) | 41.54s | 0.3725 |
| 27 | [Databend](https://github.com/ClickHouse/ClickBench/tree/main/databend) | 42.31s | 0.2095 |
| 28 | [DataFusion (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-partitioned) | 42.32s | 0.2740 |
| 29 | [Spice.ai OSS (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet) | 42.33s | 0.3805 |
| 30 | [StarRocks](https://github.com/ClickHouse/ClickBench/tree/main/starrocks) | 42.96s | 0.3076 |
| 31 | [ClickHouse (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet-partitioned) | 43.59s | 0.3175 |
| 32 | [Parseable (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/parseable) | 45.13s | 0.4011 |
| 33 | [DataFusion (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion) | 45.92s | 0.3556 |
| 34 | [VeloDB](https://github.com/ClickHouse/ClickBench/tree/main/velodb) | 46.49s | 0.4168 |
| 35 | [ClickHouse (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet) | 48.05s | 0.4264 |
| 36 | [Doris](https://github.com/ClickHouse/ClickBench/tree/main/doris) | 48.43s | 0.2752 |
| 37 | [Firebolt (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet) | 49.47s | 0.2849 |
| 38 | [DataFusion (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex) | 49.65s | 0.3783 |
| 39 | [chDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/chdb-parquet-partitioned) | 50.66s | 0.4226 |
| 40 | [ByConity](https://github.com/ClickHouse/ClickBench/tree/main/byconity) | 51.59s | 0.3476 |
| 41 | [pg_duckdb (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb-parquet) | 52.49s | 0.6511 |
| 42 | [Sail (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/sail) | 53.86s | 0.4612 |
| 43 | [ParadeDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/paradedb) | 54.32s | 0.6428 |
| 44 | [Sail (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/sail-partitioned) | 55.45s | 0.5438 |
| 45 | [pgpro_tam](https://github.com/ClickHouse/ClickBench/tree/main/pgpro_tam) | 56.07s | 0.5029 |
| 46 | [ParadeDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/paradedb-partitioned) | 56.16s | 0.6540 |
| 47 | [Salesforce Hyper (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/hyper-parquet) | 58.08s | 0.8562 |
| 48 | [ClickHouse (web)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-web) | 58.92s | 0.5170 |
| 49 | [Doris (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/doris-parquet) | 59.18s | 0.5725 |
| 50 | [pg_mooncake](https://github.com/ClickHouse/ClickBench/tree/main/pg_mooncake) | 61.01s | 0.7836 |
| 51 | [Firebolt (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet-partitioned) | 68.74s | 0.3812 |
| 52 | [DuckDB (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake) | 75.92s | 1.1593 |
| 53 | [ZigHouse](https://github.com/ClickHouse/ClickBench/tree/main/zighouse) | 77.75s | 0.0308 |
| 54 | [GlareDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb) | 80.69s | 0.7626 |
| 55 | [Daft (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet-partitioned) | 85.61s | 0.6215 |
| 56 | [GenDB](https://github.com/ClickHouse/ClickBench/tree/main/gendb) | 86.29s | 0.3599 |
| 57 | [Daft (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet) | 87.96s | 0.6743 |
| 58 | [GlareDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb-partitioned) | 90.96s | 0.7957 |
| 59 | [ClickHouse (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake-partitioned) | 96.23s | 1.4089 |
| 60 | [DuckDB (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake-partitioned) | 96.89s | 1.6478 |
| 61 | [ClickHouse (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake) | 111.83s | 1.4331 |
| 62 | [Umbra](https://github.com/ClickHouse/ClickBench/tree/main/umbra) | 154.66s | 0.0652 |
| 63 | [WarehousePG](https://github.com/ClickHouse/ClickBench/tree/main/warehousepg) | 191.96s | 1.5536 |
| 64 | [Trino (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-partitioned) | 195.99s | 3.0058 |
| 65 | [Trino (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake-partitioned) | 197.71s | 3.5135 |
| 66 | [Spark (Velox)](https://github.com/ClickHouse/ClickBench/tree/main/spark-velox) | 225.31s | 4.7481 |
| 67 | [Spark (Gluten-on-Velox)](https://github.com/ClickHouse/ClickBench/tree/main/spark-gluten) | 226.96s | 4.7788 |
| 68 | [Trino (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake) | 250.01s | 4.9152 |
| 69 | [Trino (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino) | 258.49s | 4.4425 |
| 70 | [Spark (Auron)](https://github.com/ClickHouse/ClickBench/tree/main/spark-auron) | 265.25s | 5.3076 |
| 71 | [Oxla](https://github.com/ClickHouse/ClickBench/tree/main/oxla) | 266.47s | 0.2954 |
| 72 | [OpenGPDB](https://github.com/ClickHouse/ClickBench/tree/main/opengpdb) | 281.87s | 2.1268 |
| 73 | [Greengage](https://github.com/ClickHouse/ClickBench/tree/main/greengage) | 285.76s | 2.1794 |
| 74 | [Cloudberry](https://github.com/ClickHouse/ClickBench/tree/main/cloudberry) | 294.75s | 2.0255 |
| 75 | [Spark (Comet)](https://github.com/ClickHouse/ClickBench/tree/main/spark-comet) | 309.24s | 6.6422 |
| 76 | [Spark](https://github.com/ClickHouse/ClickBench/tree/main/spark) | 332.37s | 6.3489 |
| 77 | [Impala (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/impala) | 334.32s | 3.1489 |
| 78 | [Hydra](https://github.com/ClickHouse/ClickBench/tree/main/hydra) | 478.98s | 2.8452 |
| 79 | [TimescaleDB](https://github.com/ClickHouse/ClickBench/tree/main/timescaledb) | 569.40s | 1.6139 |
| 80 | [Greenplum](https://github.com/ClickHouse/ClickBench/tree/main/greenplum) | 768.60s | 5.8524 |
| 81 | [Citus](https://github.com/ClickHouse/ClickBench/tree/main/citus) | 1745.08s | 21.0748 |
| 82 | [BQN](https://github.com/ClickHouse/ClickBench/tree/main/bqn) | 2926.84s | 14.9506 |
| 83 | [pg_duckdb (with indexes)](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb-indexed) | 3589.08s | 6.1851 |
| 84 | [PostgreSQL (with indexes)](https://github.com/ClickHouse/ClickBench/tree/main/postgresql-indexed) | 4085.54s | 0.2984 |
| 85 | [TimescaleDB (no columnstore)](https://github.com/ClickHouse/ClickBench/tree/main/timescaledb-no-columnstore) | 8826.67s | 43.0801 |
| 86 | [Hive (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/hive) | 9385.44s | 65.1297 |
| 87 | [CockroachDB](https://github.com/ClickHouse/ClickBench/tree/main/cockroachdb) | 10965.71s | 240.3620 |
| 88 | [pg_duckdb](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb) | 11524.80s | 267.2732 |
| 89 | [PostgreSQL](https://github.com/ClickHouse/ClickBench/tree/main/postgresql) | 11895.86s | 273.7589 |
| 90 | [MySQL (MyISAM)](https://github.com/ClickHouse/ClickBench/tree/main/mysql-myisam) | 18319.84s | 89.4183 |
| 91 | [MongoDB](https://github.com/ClickHouse/ClickBench/tree/main/mongodb) | 19430.23s | 29.0712 |
| 92 | [PostgreSQL (OrioleDB)](https://github.com/ClickHouse/ClickBench/tree/main/postgresql-orioledb) | 20065.08s | 460.5000 |
| 93 | [MySQL](https://github.com/ClickHouse/ClickBench/tree/main/mysql) | 21962.32s | 206.4157 |
| 94 | [Yugabyte](https://github.com/ClickHouse/ClickBench/tree/main/yugabytedb) | 51701.64s | 1190.6434 |
