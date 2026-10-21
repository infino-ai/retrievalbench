# ClickBench full comparison (c6a.4xlarge, 100M rows)

Every self-hosted engine on the ClickBench board with a complete c6a.4xlarge run, ranked by hot-run total across the 43 queries. infino sits at **#34 of 129**.

Ranks use the **newest complete run per system**: each system's most recent upstream run with all 43 queries (hot = min of tries 2 and 3), applied to infino's row too. Snapshot: upstream [ClickBench](https://github.com/ClickHouse/ClickBench) at `57db2a0`. All numbers are from upstream ClickBench; each system links to its folder there, where infino now has published results too. Managed cloud warehouses (Snowflake, Databricks, BigQuery, Redshift, and similar) are excluded because they do not run on c6a.4xlarge. See the [README](README.md) for the headline comparison and sources.

| # | System | Hot sum | Hot geomean |
|--:|---|--:|--:|
| 1 | [intent-gizmosql](https://github.com/ClickHouse/ClickBench/tree/main/intent-gizmosql) | 3.72s | 0.0229 |
| 2 | [elosdb](https://github.com/ClickHouse/ClickBench/tree/main/elosdb) | 5.21s | 0.0342 |
| 3 | [Umbra](https://github.com/ClickHouse/ClickBench/tree/main/umbra) | 5.43s | 0.0318 |
| 4 | [WaveDB](https://github.com/ClickHouse/ClickBench/tree/main/wavedb) | 9.33s | 0.0666 |
| 5 | [Pivotlake (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/pivot-parquet-partitioned) | 10.60s | 0.0782 |
| 6 | [Pivotlake (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/pivot-parquet) | 10.64s | 0.0797 |
| 7 | [Rayforce](https://github.com/ClickHouse/ClickBench/tree/main/rayforce) | 12.80s | 0.0533 |
| 8 | [ClickHouse (web)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-web) | 15.49s | 0.0887 |
| 9 | [Firebolt](https://github.com/ClickHouse/ClickBench/tree/main/firebolt) | 15.57s | 0.1305 |
| 10 | [ClickHouse](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse) | 16.03s | 0.0933 |
| 11 | [chDB](https://github.com/ClickHouse/ClickBench/tree/main/chdb) | 18.61s | 0.1348 |
| 12 | [Salesforce Hyper (web)](https://github.com/ClickHouse/ClickBench/tree/main/hyper-web) | 20.31s | 0.0736 |
| 13 | [Opteryx (`opteryx-parquet`)](https://github.com/ClickHouse/ClickBench/tree/main/opteryx-parquet) | 20.31s | 0.1367 |
| 14 | [Salesforce Hyper](https://github.com/ClickHouse/ClickBench/tree/main/hyper) | 20.50s | 0.0743 |
| 15 | [GizmoSQL](https://github.com/ClickHouse/ClickBench/tree/main/gizmosql) | 21.03s | 0.1042 |
| 16 | [DuckFlight (Flight SQL)](https://github.com/ClickHouse/ClickBench/tree/main/duckflight) | 21.08s | 0.1093 |
| 17 | [DuckDB (memory)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-memory) | 22.08s | 0.1186 |
| 18 | [ClickHouse (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet-partitioned) | 23.16s | 0.2184 |
| 19 | [MariaDB (DuckDB)](https://github.com/ClickHouse/ClickBench/tree/main/mariadb-duckdb) | 23.38s | 0.1126 |
| 20 | [Arc](https://github.com/ClickHouse/ClickBench/tree/main/arc) | 24.90s | 0.2114 |
| 21 | [QuestDB](https://github.com/ClickHouse/ClickBench/tree/main/questdb) | 25.95s | 0.1433 |
| 22 | [Ursa](https://github.com/ClickHouse/ClickBench/tree/main/ursa) | 26.15s | 0.1248 |
| 23 | [DuckDB](https://github.com/ClickHouse/ClickBench/tree/main/duckdb) | 26.25s | 0.2287 |
| 24 | [pg_deltax](https://github.com/ClickHouse/ClickBench/tree/main/pg_deltax) | 26.88s | 0.2419 |
| 25 | [DuckDB (Vortex, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex-partitioned) | 26.91s | 0.1489 |
| 26 | [ClickHouse (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet) | 27.11s | 0.3105 |
| 27 | [chDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/chdb-parquet-partitioned) | 27.86s | 0.2744 |
| 28 | [CedarDB (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/cedardb-parquet) | 28.18s | 0.3502 |
| 29 | [CedarDB](https://github.com/ClickHouse/ClickBench/tree/main/cedardb) | 28.99s | 0.0577 |
| 30 | [Parseable (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/parseable) | 29.58s | 0.2861 |
| 31 | [DuckDB (Vortex, single, load threads=2)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex-tuned) | 29.70s | 0.2851 |
| 32 | [Opteryx (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/opteryx) | 30.04s | 0.2681 |
| 33 | [ScramDB](https://github.com/ClickHouse/ClickBench/tree/main/scramdb) | 30.48s | 0.1741 |
| **34** | [**infino**](results/infino/c6a.4xlarge.json) | **30.52s** | **0.2081** |
| 35 | [Salesforce Hyper (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/hyper-parquet-single) | 30.78s | 0.2596 |
| 36 | [DuckDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet-partitioned) | 30.95s | 0.2662 |
| 37 | [DuckDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet) | 32.48s | 0.3446 |
| 38 | [Salesforce Hyper (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/hyper-parquet-partitioned) | 33.41s | 0.2878 |
| 39 | [pg_clickhouse](https://github.com/ClickHouse/ClickBench/tree/main/pg_clickhouse) | 33.60s | 0.1548 |
| 40 | [ClickHouse (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake) | 33.95s | 0.3877 |
| 41 | [Polars (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/polars-dataframe) | 34.12s | 0.2344 |
| 42 | [Spice.ai OSS (Cayenne)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-cayenne) | 34.65s | 0.2633 |
| 43 | [DataFusion (Vortex, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex-partitioned) | 35.79s | 0.2365 |
| 44 | [CtrlB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/ctrlb) | 36.41s | 0.3140 |
| 45 | [ClickHouse (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake-partitioned) | 37.25s | 0.4089 |
| 46 | [Polars (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/polars) | 37.37s | 0.2334 |
| 47 | [Opteryx (`opteryx-skene`)](https://github.com/ClickHouse/ClickBench/tree/main/opteryx-skene) | 39.23s | 0.4837 |
| 48 | [Spice.ai OSS (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet-partitioned) | 40.01s | 0.3824 |
| 49 | [DuckDB (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex) | 40.99s | 0.3269 |
| 50 | [pg_ducklake](https://github.com/ClickHouse/ClickBench/tree/main/pg_ducklake) | 41.54s | 0.3725 |
| 51 | [DataFusion (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-partitioned) | 41.87s | 0.2690 |
| 52 | [Databend](https://github.com/ClickHouse/ClickBench/tree/main/databend) | 41.93s | 0.2089 |
| 53 | [Spice.ai OSS (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet) | 42.56s | 0.3897 |
| 54 | [StarRocks](https://github.com/ClickHouse/ClickBench/tree/main/starrocks) | 44.47s | 0.3328 |
| 55 | [Umbra (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/umbra-parquet) | 44.72s | 0.1873 |
| 56 | [DataFusion (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion) | 45.57s | 0.3558 |
| 57 | [Umbra (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/umbra-parquet-partitioned) | 46.37s | 0.2122 |
| 58 | [VeloDB](https://github.com/ClickHouse/ClickBench/tree/main/velodb) | 46.49s | 0.4168 |
| 59 | [Doris](https://github.com/ClickHouse/ClickBench/tree/main/doris) | 48.43s | 0.3795 |
| 60 | [Firebolt (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet) | 49.09s | 0.4823 |
| 61 | [Sail (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/sail) | 51.49s | 0.3772 |
| 62 | [ByConity](https://github.com/ClickHouse/ClickBench/tree/main/byconity) | 51.59s | 0.3476 |
| 63 | [pg_duckdb (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb-parquet) | 52.49s | 0.6511 |
| 64 | [Sail (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/sail-partitioned) | 53.81s | 0.4506 |
| 65 | [pgpro_tam](https://github.com/ClickHouse/ClickBench/tree/main/pgpro_tam) | 56.07s | 0.5029 |
| 66 | [ParadeDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/paradedb-partitioned) | 56.16s | 0.6540 |
| 67 | [Ravel](https://github.com/ClickHouse/ClickBench/tree/main/ravel) | 58.88s | 0.7526 |
| 68 | [Doris (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/doris-parquet) | 59.18s | 0.5725 |
| 69 | [pg_mooncake](https://github.com/ClickHouse/ClickBench/tree/main/pg_mooncake) | 61.01s | 0.7836 |
| 70 | [Keyten (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/keyten-parquet) | 64.34s | 0.5639 |
| 71 | [Doris (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/doris-parquet-single) | 69.02s | 0.8340 |
| 72 | [Firebolt (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet-partitioned) | 69.99s | 0.5769 |
| 73 | [ZigHouse](https://github.com/ClickHouse/ClickBench/tree/main/zighouse) | 77.75s | 0.1536 |
| 74 | [DuckDB (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake) | 80.81s | 1.2473 |
| 75 | [GlareDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb) | 80.92s | 0.7615 |
| 76 | [Velox (Axiom)](https://github.com/ClickHouse/ClickBench/tree/main/velox) | 81.51s | 1.2012 |
| 77 | [GenDB](https://github.com/ClickHouse/ClickBench/tree/main/gendb) | 86.29s | 0.4962 |
| 78 | [Kinetica](https://github.com/ClickHouse/ClickBench/tree/main/kinetica) | 86.44s | 0.4269 |
| 79 | [Daft (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet) | 87.96s | 0.6743 |
| 80 | [Daft (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet-partitioned) | 90.14s | 0.6703 |
| 81 | [DataFusion (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex) | 91.30s | 0.4856 |
| 82 | [GlareDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb-partitioned) | 91.62s | 0.8097 |
| 83 | [Pinot (with star-tree index)](https://github.com/ClickHouse/ClickBench/tree/main/pinot-tuned) | 98.24s | 0.5507 |
| 84 | [Pinot](https://github.com/ClickHouse/ClickBench/tree/main/pinot) | 101.46s | 0.6586 |
| 85 | [DuckDB (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake-partitioned) | 116.88s | 2.1774 |
| 86 | [StarRocks (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/starrocks-parquet) | 124.04s | 1.4529 |
| 87 | [StarRocks (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/starrocks-parquet-partitioned) | 127.38s | 1.5757 |
| 88 | [MonetDB](https://github.com/ClickHouse/ClickBench/tree/main/monetdb) | 180.18s | 0.6918 |
| 89 | [WarehousePG](https://github.com/ClickHouse/ClickBench/tree/main/warehousepg) | 191.96s | 1.5536 |
| 90 | [Trino (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-partitioned) | 195.99s | 3.0058 |
| 91 | [Spark (Velox)](https://github.com/ClickHouse/ClickBench/tree/main/spark-velox) | 225.31s | 4.7481 |
| 92 | [Spark (Gluten-on-Velox)](https://github.com/ClickHouse/ClickBench/tree/main/spark-gluten) | 226.96s | 4.7788 |
| 93 | [BemiDB](https://github.com/ClickHouse/ClickBench/tree/main/bemidb) | 234.93s | 2.1521 |
| 94 | [Trino (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino) | 258.49s | 4.4425 |
| 95 | [Spark (Auron)](https://github.com/ClickHouse/ClickBench/tree/main/spark-auron) | 265.25s | 5.3076 |
| 96 | [Oxla](https://github.com/ClickHouse/ClickBench/tree/main/oxla) | 266.47s | 0.2954 |
| 97 | [OpenGPDB](https://github.com/ClickHouse/ClickBench/tree/main/opengpdb) | 281.87s | 2.1268 |
| 98 | [Greengage](https://github.com/ClickHouse/ClickBench/tree/main/greengage) | 285.76s | 2.1794 |
| 99 | [openGauss (column store)](https://github.com/ClickHouse/ClickBench/tree/main/opengauss-column) | 286.30s | 1.4270 |
| 100 | [Cloudberry](https://github.com/ClickHouse/ClickBench/tree/main/cloudberry) | 294.75s | 2.0255 |
| 101 | [Spark (Comet)](https://github.com/ClickHouse/ClickBench/tree/main/spark-comet) | 309.24s | 6.6422 |
| 102 | [Trino (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake-partitioned) | 326.44s | 6.0798 |
| 103 | [Spark](https://github.com/ClickHouse/ClickBench/tree/main/spark) | 332.37s | 6.3489 |
| 104 | [Impala (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/impala) | 334.32s | 3.1489 |
| 105 | [Trino (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake) | 355.68s | 6.6097 |
| 106 | [Hydra](https://github.com/ClickHouse/ClickBench/tree/main/hydra) | 478.98s | 2.8452 |
| 107 | [TimescaleDB](https://github.com/ClickHouse/ClickBench/tree/main/timescaledb) | 569.40s | 1.6139 |
| 108 | [Regolith](https://github.com/ClickHouse/ClickBench/tree/main/regolith) | 627.62s | 11.8940 |
| 109 | [Impala (Kudu, single)](https://github.com/ClickHouse/ClickBench/tree/main/impala-kudu) | 746.27s | 6.5384 |
| 110 | [Greenplum](https://github.com/ClickHouse/ClickBench/tree/main/greenplum) | 768.60s | 5.8524 |
| 111 | [SlateDB](https://github.com/ClickHouse/ClickBench/tree/main/slatedb) | 903.41s | 17.0284 |
| 112 | [DuckDB (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-dataframe) | 1673.40s | 0.5005 |
| 113 | [Citus](https://github.com/ClickHouse/ClickBench/tree/main/citus) | 1745.08s | 21.0748 |
| 114 | [ArcticDB](https://github.com/ClickHouse/ClickBench/tree/main/arcticdb) | 1934.56s | 3.5818 |
| 115 | [OceanBase (row store)](https://github.com/ClickHouse/ClickBench/tree/main/oceanbase-row) | 2377.63s | 14.4295 |
| 116 | [BQN](https://github.com/ClickHouse/ClickBench/tree/main/bqn) | 2926.84s | 14.9506 |
| 117 | [pg_duckdb (with indexes)](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb-indexed) | 3589.08s | 6.1851 |
| 118 | [PostgreSQL (with indexes)](https://github.com/ClickHouse/ClickBench/tree/main/postgresql-indexed) | 4085.54s | 2.0514 |
| 119 | [TimescaleDB (no columnstore)](https://github.com/ClickHouse/ClickBench/tree/main/timescaledb-no-columnstore) | 8826.67s | 43.0801 |
| 120 | [Hive (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/hive) | 9385.44s | 65.1297 |
| 121 | [CockroachDB](https://github.com/ClickHouse/ClickBench/tree/main/cockroachdb) | 10965.71s | 240.3620 |
| 122 | [pg_duckdb](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb) | 11524.80s | 267.2732 |
| 123 | [ParadeDB](https://github.com/ClickHouse/ClickBench/tree/main/paradedb) | 11822.57s | 271.0998 |
| 124 | [PostgreSQL](https://github.com/ClickHouse/ClickBench/tree/main/postgresql) | 11875.43s | 273.6017 |
| 125 | [MySQL (MyISAM)](https://github.com/ClickHouse/ClickBench/tree/main/mysql-myisam) | 18319.84s | 123.2994 |
| 126 | [MongoDB](https://github.com/ClickHouse/ClickBench/tree/main/mongodb) | 19430.23s | 29.0712 |
| 127 | [PostgreSQL (OrioleDB)](https://github.com/ClickHouse/ClickBench/tree/main/postgresql-orioledb) | 20065.08s | 460.5000 |
| 128 | [MySQL](https://github.com/ClickHouse/ClickBench/tree/main/mysql) | 21962.32s | 206.4157 |
| 129 | [Yugabyte](https://github.com/ClickHouse/ClickBench/tree/main/yugabytedb) | 51701.64s | 1190.6434 |
