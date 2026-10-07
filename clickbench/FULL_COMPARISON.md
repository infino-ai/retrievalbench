# ClickBench full comparison (c6a.4xlarge, 100M rows)

Every self-hosted engine on the ClickBench board with a complete c6a.4xlarge run, ranked by hot-run total across the 43 queries. infino sits at **#39 of 118**.

All numbers are sourced from upstream [ClickBench](https://github.com/ClickHouse/ClickBench); each system links to its folder there, where infino now has published results too. Managed cloud warehouses (Snowflake, Databricks, BigQuery, Redshift, and similar) are excluded because they do not run on c6a.4xlarge. See the [README](README.md) for the headline comparison, methodology, and sources.

| # | System | Hot sum | Hot geomean |
|--:|---|--:|--:|
| 1 | [elosdb](https://github.com/ClickHouse/ClickBench/tree/main/elosdb) | 5.21s | 0.0342 |
| 2 | [Umbra](https://github.com/ClickHouse/ClickBench/tree/main/umbra) | 7.38s | 0.0375 |
| 3 | [intent-gizmosql](https://github.com/ClickHouse/ClickBench/tree/main/intent-gizmosql) | 7.96s | 0.0314 |
| 4 | [Firebolt](https://github.com/ClickHouse/ClickBench/tree/main/firebolt) | 15.57s | 0.1305 |
| 5 | [ClickHouse (web)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-web) | 16.73s | 0.1063 |
| 6 | [ClickHouse](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse) | 17.53s | 0.1022 |
| 7 | [Salesforce Hyper](https://github.com/ClickHouse/ClickBench/tree/main/hyper) | 19.96s | 0.0693 |
| 8 | [Salesforce Hyper (web)](https://github.com/ClickHouse/ClickBench/tree/main/hyper-web) | 20.31s | 0.0736 |
| 9 | [GizmoSQL](https://github.com/ClickHouse/ClickBench/tree/main/gizmosql) | 21.03s | 0.1042 |
| 10 | [DuckDB](https://github.com/ClickHouse/ClickBench/tree/main/duckdb) | 21.76s | 0.0948 |
| 11 | [DuckDB (memory)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-memory) | 21.84s | 0.1109 |
| 12 | [chDB](https://github.com/ClickHouse/ClickBench/tree/main/chdb) | 22.58s | 0.2462 |
| 13 | [MariaDB (DuckDB)](https://github.com/ClickHouse/ClickBench/tree/main/mariadb-duckdb) | 22.91s | 0.1092 |
| 14 | [Umbra (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/umbra-parquet) | 23.94s | 0.1826 |
| 15 | [Arc](https://github.com/ClickHouse/ClickBench/tree/main/arc) | 24.65s | 0.2081 |
| 16 | [Umbra (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/umbra-parquet-partitioned) | 25.61s | 0.2055 |
| 17 | [Ursa](https://github.com/ClickHouse/ClickBench/tree/main/ursa) | 26.10s | 0.1292 |
| 18 | [ClickHouse (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet-partitioned) | 26.15s | 0.2467 |
| 19 | [pg_deltax](https://github.com/ClickHouse/ClickBench/tree/main/pg_deltax) | 26.88s | 0.2419 |
| 20 | [DuckDB (Vortex, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex-partitioned) | 26.91s | 0.1489 |
| 21 | [CedarDB (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/cedardb-parquet) | 28.18s | 0.3502 |
| 22 | [DuckDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet) | 28.27s | 0.1839 |
| 23 | [ClickHouse (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-parquet) | 28.39s | 0.3218 |
| 24 | [pg_ducklake](https://github.com/ClickHouse/ClickBench/tree/main/pg_ducklake) | 28.84s | 0.2181 |
| 25 | [CedarDB](https://github.com/ClickHouse/ClickBench/tree/main/cedardb) | 28.99s | 0.0577 |
| 26 | [DuckDB (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake) | 29.17s | 0.1938 |
| 27 | [DuckDB (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-datalake-partitioned) | 29.17s | 0.1938 |
| 28 | [pg_clickhouse](https://github.com/ClickHouse/ClickBench/tree/main/pg_clickhouse) | 29.20s | 0.1598 |
| 29 | [Parseable (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/parseable) | 29.58s | 0.2861 |
| 30 | [DuckDB (Vortex, single, load threads=2)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex-tuned) | 29.70s | 0.2851 |
| 31 | [DuckDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-parquet-partitioned) | 29.83s | 0.2123 |
| 32 | [ScramDB](https://github.com/ClickHouse/ClickBench/tree/main/scramdb) | 30.48s | 0.1741 |
| 33 | [Salesforce Hyper (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/hyper-parquet-single) | 30.77s | 0.2645 |
| 34 | [DuckDB (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-vortex) | 31.38s | 0.2225 |
| 35 | [Salesforce Hyper (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/hyper-parquet-partitioned) | 32.66s | 0.2953 |
| 36 | [VeloDB](https://github.com/ClickHouse/ClickBench/tree/main/velodb) | 32.92s | 0.1997 |
| 37 | [Spice.ai OSS (Cayenne)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-cayenne) | 33.12s | 0.2325 |
| 38 | [Polars (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/polars-dataframe) | 33.20s | 0.2288 |
| **39** | [**infino**](results/infino/c6a.4xlarge.json) | **33.74s** | **0.2636** |
| 40 | [ClickHouse (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake) | 34.55s | 0.4115 |
| 41 | [DataFusion (Vortex, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex-partitioned) | 35.79s | 0.2365 |
| 42 | [CtrlB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/ctrlb) | 36.17s | 0.3088 |
| 43 | [Polars (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/polars) | 37.27s | 0.2740 |
| 44 | [chDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/chdb-parquet-partitioned) | 38.82s | 0.3960 |
| 45 | [Opteryx](https://github.com/ClickHouse/ClickBench/tree/main/opteryx-skene) | 39.23s | 0.4837 |
| 46 | [ClickHouse (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/clickhouse-datalake-partitioned) | 39.38s | 0.4371 |
| 47 | [Spice.ai OSS (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet-partitioned) | 40.01s | 0.3824 |
| 48 | [StarRocks](https://github.com/ClickHouse/ClickBench/tree/main/starrocks) | 40.30s | 0.2981 |
| 49 | [Doris](https://github.com/ClickHouse/ClickBench/tree/main/doris) | 40.55s | 0.2543 |
| 50 | [Opteryx (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/opteryx) | 41.25s | 0.3328 |
| 51 | [Databend](https://github.com/ClickHouse/ClickBench/tree/main/databend) | 41.75s | 0.1818 |
| 52 | [DataFusion (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-partitioned) | 41.87s | 0.2690 |
| 53 | [Spice.ai OSS (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/spiceai-parquet) | 42.24s | 0.3622 |
| 54 | [DataFusion (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion) | 45.57s | 0.3558 |
| 55 | [DataFusion (Vortex, single)](https://github.com/ClickHouse/ClickBench/tree/main/datafusion-vortex) | 46.83s | 0.3567 |
| 56 | [Firebolt (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet) | 48.98s | 0.2828 |
| 57 | [pg_duckdb (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb-parquet) | 50.99s | 0.5964 |
| 58 | [ByConity](https://github.com/ClickHouse/ClickBench/tree/main/byconity) | 51.59s | 0.3476 |
| 59 | [Sail (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/sail-partitioned) | 52.55s | 0.4512 |
| 60 | [pg_mooncake](https://github.com/ClickHouse/ClickBench/tree/main/pg_mooncake) | 52.92s | 0.4538 |
| 61 | [Sail (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/sail) | 53.86s | 0.4612 |
| 62 | [ParadeDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/paradedb) | 54.32s | 0.6428 |
| 63 | [pgpro_tam](https://github.com/ClickHouse/ClickBench/tree/main/pgpro_tam) | 54.43s | 0.5000 |
| 64 | [Doris (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/doris-parquet) | 55.73s | 0.4540 |
| 65 | [ParadeDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/paradedb-partitioned) | 56.16s | 0.6540 |
| 66 | [Keyten (Parquet)](https://github.com/ClickHouse/ClickBench/tree/main/keyten-parquet) | 64.34s | 0.5639 |
| 67 | [Firebolt (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/firebolt-parquet-partitioned) | 68.06s | 0.3957 |
| 68 | [Doris (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/doris-parquet-single) | 69.02s | 0.8340 |
| 69 | [pg_duckdb](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb) | 74.95s | 0.7924 |
| 70 | [GlareDB (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb) | 80.48s | 0.7598 |
| 71 | [Velox (Axiom)](https://github.com/ClickHouse/ClickBench/tree/main/velox) | 81.51s | 1.2012 |
| 72 | [Daft (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet-partitioned) | 85.61s | 0.6215 |
| 73 | [Kinetica](https://github.com/ClickHouse/ClickBench/tree/main/kinetica) | 86.44s | 0.4269 |
| 74 | [Ravel](https://github.com/ClickHouse/ClickBench/tree/main/ravel) | 87.21s | 1.5571 |
| 75 | [Daft (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/daft-parquet) | 87.96s | 0.6743 |
| 76 | [GlareDB (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/glaredb-partitioned) | 90.96s | 0.7957 |
| 77 | [Pinot (with star-tree index)](https://github.com/ClickHouse/ClickBench/tree/main/pinot-tuned) | 98.24s | 0.5507 |
| 78 | [Pinot](https://github.com/ClickHouse/ClickBench/tree/main/pinot) | 100.99s | 0.6509 |
| 79 | [StarRocks (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/starrocks-parquet) | 120.82s | 1.4183 |
| 80 | [Spark (Gluten-on-Velox)](https://github.com/ClickHouse/ClickBench/tree/main/spark-gluten) | 124.76s | 2.1990 |
| 81 | [StarRocks (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/starrocks-parquet-partitioned) | 124.91s | 1.4666 |
| 82 | [Spark (Auron)](https://github.com/ClickHouse/ClickBench/tree/main/spark-auron) | 168.83s | 2.6709 |
| 83 | [Trino (Parquet, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-partitioned) | 171.85s | 2.8879 |
| 84 | [Spark (Comet)](https://github.com/ClickHouse/ClickBench/tree/main/spark-comet) | 171.99s | 3.2895 |
| 85 | [WarehousePG](https://github.com/ClickHouse/ClickBench/tree/main/warehousepg) | 191.96s | 1.5536 |
| 86 | [Trino (data lake, partitioned)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake-partitioned) | 192.77s | 3.3250 |
| 87 | [Spark](https://github.com/ClickHouse/ClickBench/tree/main/spark) | 197.85s | 2.8126 |
| 88 | [Spark (Velox)](https://github.com/ClickHouse/ClickBench/tree/main/spark-velox) | 224.75s | 4.7377 |
| 89 | [BemiDB](https://github.com/ClickHouse/ClickBench/tree/main/bemidb) | 234.93s | 2.1521 |
| 90 | [Trino (data lake, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino-datalake) | 242.09s | 4.6382 |
| 91 | [Trino (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/trino) | 248.63s | 4.1863 |
| 92 | [Oxla](https://github.com/ClickHouse/ClickBench/tree/main/oxla) | 263.82s | 0.2912 |
| 93 | [OpenGPDB](https://github.com/ClickHouse/ClickBench/tree/main/opengpdb) | 281.87s | 2.1268 |
| 94 | [Greengage](https://github.com/ClickHouse/ClickBench/tree/main/greengage) | 285.76s | 2.1794 |
| 95 | [openGauss (column store)](https://github.com/ClickHouse/ClickBench/tree/main/opengauss-column) | 286.30s | 1.4270 |
| 96 | [Greenplum](https://github.com/ClickHouse/ClickBench/tree/main/greenplum) | 287.31s | 2.1535 |
| 97 | [Cloudberry](https://github.com/ClickHouse/ClickBench/tree/main/cloudberry) | 294.75s | 2.0255 |
| 98 | [Impala (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/impala) | 334.32s | 3.1489 |
| 99 | [Hydra](https://github.com/ClickHouse/ClickBench/tree/main/hydra) | 478.98s | 2.8452 |
| 100 | [TimescaleDB](https://github.com/ClickHouse/ClickBench/tree/main/timescaledb) | 491.45s | 1.4939 |
| 101 | [Regolith](https://github.com/ClickHouse/ClickBench/tree/main/regolith) | 627.62s | 11.8940 |
| 102 | [Impala (Kudu, single)](https://github.com/ClickHouse/ClickBench/tree/main/impala-kudu) | 746.27s | 6.5384 |
| 103 | [SlateDB](https://github.com/ClickHouse/ClickBench/tree/main/slatedb) | 903.41s | 17.0284 |
| 104 | [DuckDB (DataFrame)](https://github.com/ClickHouse/ClickBench/tree/main/duckdb-dataframe) | 1673.40s | 0.5005 |
| 105 | [Citus](https://github.com/ClickHouse/ClickBench/tree/main/citus) | 1715.50s | 14.4242 |
| 106 | [ArcticDB](https://github.com/ClickHouse/ClickBench/tree/main/arcticdb) | 1934.56s | 3.5818 |
| 107 | [OceanBase (row store)](https://github.com/ClickHouse/ClickBench/tree/main/oceanbase-row) | 2377.63s | 14.4295 |
| 108 | [BQN](https://github.com/ClickHouse/ClickBench/tree/main/bqn) | 2926.84s | 14.9506 |
| 109 | [pg_duckdb (with indexes)](https://github.com/ClickHouse/ClickBench/tree/main/pg_duckdb-indexed) | 3589.08s | 6.1851 |
| 110 | [PostgreSQL](https://github.com/ClickHouse/ClickBench/tree/main/postgresql) | 3675.30s | 2.2987 |
| 111 | [PostgreSQL (with indexes)](https://github.com/ClickHouse/ClickBench/tree/main/postgresql-indexed) | 3675.30s | 2.2987 |
| 112 | [TimescaleDB (no columnstore)](https://github.com/ClickHouse/ClickBench/tree/main/timescaledb-no-columnstore) | 8826.67s | 43.0801 |
| 113 | [Hive (Parquet, single)](https://github.com/ClickHouse/ClickBench/tree/main/hive) | 9385.44s | 65.1297 |
| 114 | [CockroachDB](https://github.com/ClickHouse/ClickBench/tree/main/cockroachdb) | 10965.71s | 240.3620 |
| 115 | [MongoDB](https://github.com/ClickHouse/ClickBench/tree/main/mongodb) | 19430.23s | 29.0712 |
| 116 | [PostgreSQL (OrioleDB)](https://github.com/ClickHouse/ClickBench/tree/main/postgresql-orioledb) | 20065.08s | 460.5000 |
| 117 | [MySQL](https://github.com/ClickHouse/ClickBench/tree/main/mysql) | 21962.32s | 206.4157 |
| 118 | [Yugabyte](https://github.com/ClickHouse/ClickBench/tree/main/yugabytedb) | 51701.64s | 1190.6434 |
