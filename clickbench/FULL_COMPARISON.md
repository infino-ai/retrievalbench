# ClickBench full comparison (c6a.4xlarge, 100M rows)

Every self-hosted engine on the ClickBench board with a complete c6a.4xlarge run, ranked by hot-run total across the 43 queries. infino sits at **#35 of 126**.

Ranks use the **newest complete run per system**: each system's most recent upstream run with all 43 queries (hot = min of tries 2 and 3), applied to infino's row too. Snapshot: upstream [ClickBench](https://github.com/ClickHouse/ClickBench) at `6152b6b`. All numbers are from upstream ClickBench; each system links to its folder there, where infino now has published results too. Managed cloud warehouses (Snowflake, Databricks, BigQuery, Redshift, and similar) are excluded because they do not run on c6a.4xlarge. See the [README](README.md) for the headline comparison and sources.

| # | System | Hot sum | Hot geomean |
|--:|---|--:|--:|
| 1 | [elosdb](https://github.com/ClickHouse/ClickBench/tree/main/elosdb) | 5.21s | 0.0342 |
| 2 | [Umbra](https://github.com/ClickHouse/ClickBench/tree/main/umbra) | 7.41s | 0.0370 |
| 3 | [intent-gizmosql](https://github.com/ClickHouse/ClickBench/tree/main/intent-gizmosql) | 7.96s | 0.0314 |
| 4 | [Pivotlake (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/pivot-parquet-partitioned) | 10.60s | 0.0782 |
| 5 | [Pivotlake (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/pivot-parquet) | 10.64s | 0.0797 |
| 6 | [Rayforce](https://github.com/ClickHouse/ClickBench/tree/main/rayforce) | 11.60s | 0.0526 |
| 7 | [Firebolt](https://github.com/ClickHouse/ClickBench/tree/main/firebolt) | 15.57s | 0.1305 |
| 8 | [ClickHouse (web)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-web) | 17.00s | 0.1030 |
| 9 | [ClickHouse](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse) | 17.44s | 0.1039 |
| 10 | [Salesforce Hyper](https://github.com/ClickHouse/ClickBench/tree/main/hyper) | 19.96s | 0.0693 |
| 11 | [Salesforce Hyper (web)](https://github.com/ClickHouse/ClickBench/tree/main/hyper-web) | 20.31s | 0.0736 |
| 12 | [GizmoSQL](https://github.com/ClickHouse/ClickBench/tree/main/gizmosql) | 21.03s | 0.1042 |
| 13 | [DuckDB (memory)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-memory) | 22.08s | 0.1186 |
| 14 | [chDB](https://github.com/ClickHouse/ClickBench/tree/main/chdb) | 22.58s | 0.2462 |
| 15 | [MariaDB (DuckDB)](https://github.com/ClickHouse/ClickBench/tree/main/mariadb-duckdb) | 23.38s | 0.1126 |
| 16 | [Umbra (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/umbra-parquet) | 23.94s | 0.1826 |
| 17 | [Arc](https://github.com/ClickHouse/ClickBench/tree/main/arc) | 24.90s | 0.2114 |
| 18 | [Umbra (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/umbra-parquet-partitioned) | 25.61s | 0.2055 |
| 19 | [QuestDB](https://github.com/ClickHouse/ClickBench/tree/main/questdb) | 25.95s | 0.1433 |
| 20 | [Ursa](https://github.com/ClickHouse/ClickBench/tree/main/ursa) | 26.15s | 0.1248 |
| 21 | [DuckDB](https://github.com/ClickHouse/ClickBench/tree/main/duckdb) | 26.25s | 0.2287 |
| 22 | [ClickHouse (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet-partitioned) | 26.36s | 0.2538 |
| 23 | [pg_deltax](https://github.com/ClickHouse/ClickBench/tree/main/pg_deltax) | 26.88s | 0.2419 |
| 24 | [DuckDB (Vortex, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex-partitioned) | 26.91s | 0.1489 |
| 25 | [CedarDB (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/cedardb-parquet) | 28.18s | 0.3502 |
| 26 | [ClickHouse (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet) | 28.39s | 0.3218 |
| 27 | [CedarDB](https://github.com/ClickHouse/ClickBench/tree/main/cedardb) | 28.99s | 0.0577 |
| 28 | [Parseable (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/parseable) | 29.58s | 0.2861 |
| 29 | [DuckDB (Vortex, single, load threads=2)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex-tuned) | 29.70s | 0.2851 |
| 30 | [ScramDB](https://github.com/ClickHouse/ClickBench/tree/main/scramdb) | 30.48s | 0.1741 |
| 31 | [Salesforce Hyper (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/hyper-parquet-single) | 30.77s | 0.2645 |
| 32 | [DuckDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet-partitioned) | 30.95s | 0.2662 |
| 33 | [DuckDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet) | 32.48s | 0.3446 |
| 34 | [pg_clickhouse](https://github.com/ClickHouse/ClickBench/tree/main/pg_clickhouse) | 33.60s | 0.1548 |
| **35** | [**infino**](results/infino/c6a.4xlarge.json) | **33.74s** | **0.2636** |
| 36 | [Salesforce Hyper (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/hyper-parquet-partitioned) | 33.80s | 0.2897 |
| 37 | [Polars (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/polars-dataframe) | 34.12s | 0.2344 |
| 38 | [Spice.ai OSS (Cayenne)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-cayenne) | 34.65s | 0.2633 |
| 39 | [DataFusion (Vortex, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex-partitioned) | 35.79s | 0.2365 |
| 40 | [CtrlB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/ctrlb) | 36.41s | 0.3140 |
| 41 | [ClickHouse (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake) | 36.58s | 0.4173 |
| 42 | [chDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/chdb-parquet-partitioned) | 38.82s | 0.3960 |
| 43 | [Opteryx](https://github.com/ClickHouse/ClickBench/tree/main/opteryx-skene) | 39.23s | 0.4837 |
| 44 | [Spice.ai OSS (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet-partitioned) | 40.01s | 0.3824 |
| 45 | [DuckDB (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex) | 40.99s | 0.3269 |
| 46 | [Opteryx (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/opteryx) | 41.25s | 0.3328 |
| 47 | [pg_ducklake](https://github.com/ClickHouse/ClickBench/tree/main/pg_ducklake) | 41.54s | 0.3725 |
| 48 | [DataFusion (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-partitioned) | 41.87s | 0.2690 |
| 49 | [Databend](https://github.com/ClickHouse/ClickBench/tree/main/databend) | 41.93s | 0.2089 |
| 50 | [ClickHouse (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake-partitioned) | 42.27s | 0.4616 |
| 51 | [Spice.ai OSS (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet) | 42.56s | 0.3897 |
| 52 | [StarRocks](https://github.com/ClickHouse/ClickBench/tree/main/starrocks) | 44.47s | 0.3328 |
| 53 | [Polars (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/polars) | 45.35s | 0.2732 |
| 54 | [DataFusion (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion) | 45.57s | 0.3558 |
| 55 | [VeloDB](https://github.com/ClickHouse/ClickBench/tree/main/velodb) | 46.49s | 0.4168 |
| 56 | [Doris](https://github.com/ClickHouse/ClickBench/tree/main/doris) | 48.43s | 0.3795 |
| 57 | [Firebolt (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet) | 49.09s | 0.4823 |
| 58 | [ByConity](https://github.com/ClickHouse/ClickBench/tree/main/byconity) | 51.59s | 0.3476 |
| 59 | [pg_duckdb (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb-parquet) | 52.49s | 0.6511 |
| 60 | [Sail (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/sail-partitioned) | 52.55s | 0.4512 |
| 61 | [Sail (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/sail) | 54.39s | 0.4126 |
| 62 | [pgpro_tam](https://github.com/ClickHouse/ClickBench/tree/main/pgpro_tam) | 56.07s | 0.5029 |
| 63 | [ParadeDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/paradedb-partitioned) | 56.16s | 0.6540 |
| 64 | [Doris (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/doris-parquet) | 59.18s | 0.5725 |
| 65 | [pg_mooncake](https://github.com/ClickHouse/ClickBench/tree/main/pg_mooncake) | 61.01s | 0.7836 |
| 66 | [Keyten (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/keyten-parquet) | 64.34s | 0.5639 |
| 67 | [Doris (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/doris-parquet-single) | 69.02s | 0.8340 |
| 68 | [Firebolt (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet-partitioned) | 69.99s | 0.5769 |
| 69 | [ZigHouse](https://github.com/ClickHouse/ClickBench/tree/main/zighouse) | 77.75s | 0.1536 |
| 70 | [DuckDB (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake) | 80.81s | 1.2473 |
| 71 | [GlareDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb) | 80.92s | 0.7615 |
| 72 | [Velox (Axiom)](https://github.com/ClickHouse/ClickBench/tree/main/velox) | 81.51s | 1.2012 |
| 73 | [GenDB](https://github.com/ClickHouse/ClickBench/tree/main/gendb) | 86.29s | 0.4962 |
| 74 | [Kinetica](https://github.com/ClickHouse/ClickBench/tree/main/kinetica) | 86.44s | 0.4269 |
| 75 | [Ravel](https://github.com/ClickHouse/ClickBench/tree/main/ravel) | 87.21s | 1.5571 |
| 76 | [Daft (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet) | 87.96s | 0.6743 |
| 77 | [Daft (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet-partitioned) | 90.14s | 0.6703 |
| 78 | [DataFusion (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex) | 91.30s | 0.4856 |
| 79 | [GlareDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb-partitioned) | 91.62s | 0.8097 |
| 80 | [Pinot (with star-tree index)](https://github.com/ClickHouse/ClickBench/tree/main/pinot-tuned) | 98.24s | 0.5507 |
| 81 | [Pinot](https://github.com/ClickHouse/ClickBench/tree/main/pinot) | 101.46s | 0.6586 |
| 82 | [DuckDB (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake-partitioned) | 116.88s | 2.1774 |
| 83 | [StarRocks (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/starrocks-parquet) | 124.04s | 1.4529 |
| 84 | [StarRocks (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/starrocks-parquet-partitioned) | 127.38s | 1.5757 |
| 85 | [MonetDB](https://github.com/ClickHouse/ClickBench/tree/main/monetdb) | 180.18s | 0.6918 |
| 86 | [WarehousePG](https://github.com/ClickHouse/ClickBench/tree/main/warehousepg) | 191.96s | 1.5536 |
| 87 | [Trino (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-partitioned) | 195.99s | 3.0058 |
| 88 | [Spark (Velox)](https://github.com/ClickHouse/ClickBench/tree/main/spark-velox) | 225.31s | 4.7481 |
| 89 | [Spark (Gluten-on-Velox)](https://github.com/ClickHouse/ClickBench/tree/main/spark-gluten) | 226.96s | 4.7788 |
| 90 | [BemiDB](https://github.com/ClickHouse/ClickBench/tree/main/bemidb) | 234.93s | 2.1521 |
| 91 | [Trino (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino) | 258.49s | 4.4425 |
| 92 | [Spark (Auron)](https://github.com/ClickHouse/ClickBench/tree/main/spark-auron) | 265.25s | 5.3076 |
| 93 | [Oxla](https://github.com/ClickHouse/ClickBench/tree/main/oxla) | 266.47s | 0.2954 |
| 94 | [OpenGPDB](https://github.com/ClickHouse/ClickBench/tree/main/opengpdb) | 281.87s | 2.1268 |
| 95 | [Greengage](https://github.com/ClickHouse/ClickBench/tree/main/greengage) | 285.76s | 2.1794 |
| 96 | [openGauss (column store)](https://github.com/ClickHouse/ClickBench/tree/main/opengauss-column) | 286.30s | 1.4270 |
| 97 | [Cloudberry](https://github.com/ClickHouse/ClickBench/tree/main/cloudberry) | 294.75s | 2.0255 |
| 98 | [Spark (Comet)](https://github.com/ClickHouse/ClickBench/tree/main/spark-comet) | 309.24s | 6.6422 |
| 99 | [Trino (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake-partitioned) | 326.44s | 6.0798 |
| 100 | [Spark](https://github.com/ClickHouse/ClickBench/tree/main/spark) | 332.37s | 6.3489 |
| 101 | [Impala (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/impala) | 334.32s | 3.1489 |
| 102 | [Trino (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake) | 355.68s | 6.6097 |
| 103 | [Hydra](https://github.com/ClickHouse/ClickBench/tree/main/hydra) | 478.98s | 2.8452 |
| 104 | [TimescaleDB](https://github.com/ClickHouse/ClickBench/tree/main/timescaledb) | 569.40s | 1.6139 |
| 105 | [Regolith](https://github.com/ClickHouse/ClickBench/tree/main/regolith) | 627.62s | 11.8940 |
| 106 | [Impala (Kudu, single)](https://github.com/ClickHouse/ClickBench/tree/main/impala-kudu) | 746.27s | 6.5384 |
| 107 | [Greenplum](https://github.com/ClickHouse/ClickBench/tree/main/greenplum) | 768.60s | 5.8524 |
| 108 | [SlateDB](https://github.com/ClickHouse/ClickBench/tree/main/slatedb) | 903.41s | 17.0284 |
| 109 | [DuckDB (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-dataframe) | 1673.40s | 0.5005 |
| 110 | [Citus](https://github.com/ClickHouse/ClickBench/tree/main/citus) | 1745.08s | 21.0748 |
| 111 | [ArcticDB](https://github.com/ClickHouse/ClickBench/tree/main/arcticdb) | 1934.56s | 3.5818 |
| 112 | [OceanBase (row store)](https://github.com/ClickHouse/ClickBench/tree/main/oceanbase-row) | 2377.63s | 14.4295 |
| 113 | [BQN](https://github.com/ClickHouse/ClickBench/tree/main/bqn) | 2926.84s | 14.9506 |
| 114 | [pg_duckdb (with indexes)](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb-indexed) | 3589.08s | 6.1851 |
| 115 | [PostgreSQL (with indexes)](https://github.com/ClickHouse/ClickBench/tree/main/postgresql-indexed) | 4085.54s | 2.0514 |
| 116 | [TimescaleDB (no columnstore)](https://github.com/ClickHouse/ClickBench/tree/main/timescaledb-no-columnstore) | 8826.67s | 43.0801 |
| 117 | [Hive (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/hive) | 9385.44s | 65.1297 |
| 118 | [CockroachDB](https://github.com/ClickHouse/ClickBench/tree/main/cockroachdb) | 10965.71s | 240.3620 |
| 119 | [pg_duckdb](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb) | 11524.80s | 267.2732 |
| 120 | [ParadeDB](https://github.com/ClickHouse/ClickBench/tree/main/paradedb) | 11822.57s | 271.0998 |
| 121 | [PostgreSQL](https://github.com/ClickHouse/ClickBench/tree/main/postgresql) | 11875.43s | 273.6017 |
| 122 | [MySQL (MyISAM)](https://github.com/ClickHouse/ClickBench/tree/main/mysql-myisam) | 18319.84s | 123.2994 |
| 123 | [MongoDB](https://github.com/ClickHouse/ClickBench/tree/main/mongodb) | 19430.23s | 29.0712 |
| 124 | [PostgreSQL (OrioleDB)](https://github.com/ClickHouse/ClickBench/tree/main/postgresql-orioledb) | 20065.08s | 460.5000 |
| 125 | [MySQL](https://github.com/ClickHouse/ClickBench/tree/main/mysql) | 21962.32s | 206.4157 |
| 126 | [Yugabyte](https://github.com/ClickHouse/ClickBench/tree/main/yugabytedb) | 51701.64s | 1190.6434 |
