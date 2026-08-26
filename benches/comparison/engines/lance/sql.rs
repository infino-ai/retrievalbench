// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! LanceDB peer implementation of [`SqlEngine`].
//!
//! Builds a Lance dataset with the portable scalar columns the SQL
//! battery references (`title`, `category`, `rating`) and answers queries
//! through a DataFusion context backed by the live Lance table provider
//! (real lance scans with pushdown — never an in-memory copy).

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use datafusion::prelude::SessionContext;
use lancedb::Table;
use lancedb::table::datafusion::BaseTableAdapter;
use tokio::runtime::Runtime;

use super::location::{LanceLocation, LanceStorage, lance_peer_label};

use infino_bench_utils::executors::payload_bytes;
use infino_bench_utils::executors::sql::SqlRead;
use infino_bench_utils::harness::{Capabilities, SqlEngine, SqlOutput, SqlRow};

const SQL_TABLE: &str = "sql";
const SQL_VIEW: &str = "supertable";
const LANCE_TEXT_BATCH_ROWS: usize = 100_000;

fn new_runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

async fn connect(uri: &str, storage_options: &[(String, String)]) -> lancedb::Connection {
    lancedb::connect(uri)
        .storage_options(
            storage_options
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        )
        .execute()
        .await
        .expect("lancedb connect")
}

async fn open_sql_table(uri: &str, storage_options: &[(String, String)]) -> Table {
    connect(uri, storage_options)
        .await
        .open_table(SQL_TABLE)
        .execute()
        .await
        .expect("open lance sql table")
}

async fn build_sql_table(
    uri: &str,
    storage_options: &[(String, String)],
    rows: &[SqlRow<'_>],
) -> Table {
    let schema = Arc::new(Schema::new(vec![
        Field::new("doc_id", DataType::UInt64, false),
        Field::new("title", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("rating", DataType::Int64, false),
    ]));
    let db = connect(uri, storage_options).await;
    let table = db
        .create_empty_table(SQL_TABLE, schema.clone())
        .execute()
        .await
        .expect("create lance sql table");
    for chunk in rows.chunks(LANCE_TEXT_BATCH_ROWS) {
        let ids = UInt64Array::from(chunk.iter().map(|r| r.doc_id).collect::<Vec<_>>());
        let titles = StringArray::from(chunk.iter().map(|r| r.title).collect::<Vec<&str>>());
        let categories = StringArray::from(chunk.iter().map(|r| r.category).collect::<Vec<&str>>());
        let ratings = Int64Array::from(chunk.iter().map(|r| r.score).collect::<Vec<_>>());
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(ids),
                Arc::new(titles),
                Arc::new(categories),
                Arc::new(ratings),
            ],
        )
        .expect("build RecordBatch");
        table
            .add(vec![batch])
            .execute()
            .await
            .expect("add lance sql batch");
    }
    table
}

async fn register_sql_ctx(table: &Table) -> SessionContext {
    // Register the REAL lance table provider (projection/filter pushdown
    // into lance scans), not an in-memory copy: the battery must pay
    // lance's actual storage read path, exactly as infino's SQL battery
    // pays its own object-store scan path.
    let ctx = SessionContext::new();
    let adapter = BaseTableAdapter::try_new(table.base_table().clone())
        .await
        .expect("adapt lance table for datafusion");
    ctx.register_table(SQL_VIEW, Arc::new(adapter))
        .expect("register supertable provider");
    ctx
}

pub struct LanceSqlEngine;
pub struct LanceS3SqlEngine;

pub struct LanceSqlIndex {
    rt: Runtime,
    location: LanceLocation,
    table: Option<Table>,
    ctx: Option<SessionContext>,
}

impl LanceSqlIndex {
    /// Table opened on the measured 1-writer artifact.
    pub fn table(&self) -> &Table {
        self.table.as_ref().expect("table requested before write")
    }

    /// DataFusion context with the dataset registered.
    pub fn ctx(&self) -> &SessionContext {
        self.ctx.as_ref().expect("ctx requested before write")
    }
}

fn create_index(location: LanceLocation) -> LanceSqlIndex {
    LanceSqlIndex {
        rt: new_runtime(),
        location,
        table: None,
        ctx: None,
    }
}

fn write_index(index: &mut LanceSqlIndex, rows: &[SqlRow<'_>]) {
    let uri = index.location.uri.clone();
    let storage_options = index.location.storage_options.clone();
    let (table, ctx) = index.rt.block_on(async {
        let table = build_sql_table(&uri, &storage_options, rows).await;
        let ctx = register_sql_ctx(&table).await;
        (table, ctx)
    });
    index.table = Some(table);
    index.ctx = Some(ctx);
}

fn parallel_write_index(rows: &[SqlRow<'_>], writers: usize, remote: bool) {
    if writers <= 1 {
        let rt = new_runtime();
        let location = if remote {
            LanceLocation::object_store("sql")
        } else {
            LanceLocation::local()
        };
        let table = rt.block_on(build_sql_table(
            &location.uri,
            &location.storage_options,
            rows,
        ));
        std::hint::black_box(&table);
        return;
    }
    let rows_per = rows.len().div_ceil(writers);
    let built: Vec<Table> = rows
        .chunks(rows_per)
        .enumerate()
        .map(|(i, shard)| {
            let rt = new_runtime();
            let location = if remote {
                LanceLocation::object_store(&format!("sql-shard-{i}"))
            } else {
                LanceLocation::local()
            };
            rt.block_on(build_sql_table(
                &location.uri,
                &location.storage_options,
                shard,
            ))
        })
        .collect();
    std::hint::black_box(built);
}

fn read_index(index: &LanceSqlIndex, sql: &str) -> SqlOutput {
    let ctx = index.ctx();
    let rows = index.rt.block_on(async {
        let df = ctx.sql(sql).await.expect("plan sql");
        let batches = df.collect().await.expect("collect sql result");
        batches.iter().map(|b| b.num_rows()).sum::<usize>()
    });
    SqlOutput { rows }
}

impl LanceSqlIndex {
    /// Reopen the same object-store artifact and run one SQL query. Used by the
    /// comparison cold tier so cold does not include rebuild time.
    pub fn cold_read(&self, sql: &str) -> SqlOutput {
        let uri = self.location.uri.clone();
        let storage_options = self.location.storage_options.clone();
        self.rt.block_on(async {
            let table = open_sql_table(&uri, &storage_options).await;
            let ctx = register_sql_ctx(&table).await;
            let df = ctx.sql(sql).await.expect("plan cold sql");
            let batches = df.collect().await.expect("collect cold sql result");
            SqlOutput {
                rows: batches.iter().map(|b| b.num_rows()).sum(),
            }
        })
    }

    /// Open the cold object-store artifact (connect + open_table + register the scanned
    /// dataset into a DataFusion context) without running the query. The cold
    /// tier times only the query, so this hydration is excluded — matching the
    /// Infino cold path, which opens its consumer outside the timed region.
    /// Note: `register_sql_ctx` scans the table into an in-memory `MemTable`,
    /// so the subsequent `cold_query` runs in memory.
    pub fn cold_open(&self) -> SessionContext {
        let uri = self.location.uri.clone();
        let storage_options = self.location.storage_options.clone();
        self.rt.block_on(async {
            let table = open_sql_table(&uri, &storage_options).await;
            register_sql_ctx(&table).await
        })
    }

    /// Run one SQL query against an already-opened cold context (query only).
    pub fn cold_query(&self, ctx: &SessionContext, sql: &str) -> SqlOutput {
        let rows = self.rt.block_on(async {
            let df = ctx.sql(sql).await.expect("plan cold sql");
            let batches = df.collect().await.expect("collect cold sql result");
            batches.iter().map(|b| b.num_rows()).sum::<usize>()
        });
        SqlOutput { rows }
    }
}

/// One SQL query against a DataFusion context over the live lance
/// provider, returning the collected batches — the shared `SqlRead`
/// phases below all route through this.
fn sql_batches(rt: &Runtime, ctx: &SessionContext, sql: &str) -> Vec<RecordBatch> {
    rt.block_on(async {
        let df = ctx.sql(sql).await.expect("plan sql");
        df.collect().await.expect("collect sql result")
    })
}

pub(crate) fn scalar_i64(batches: &[RecordBatch]) -> i64 {
    batches
        .first()
        .expect("scalar result batch")
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("i64 scalar column")
        .value(0)
}

impl SqlRead for LanceSqlIndex {
    fn query_rows(&self, sql: &str) -> usize {
        sql_batches(&self.rt, self.ctx(), sql)
            .iter()
            .map(|b| b.num_rows())
            .sum()
    }

    fn query_payload(&self, sql: &str) -> (u64, u64) {
        payload_bytes(&sql_batches(&self.rt, self.ctx(), sql))
    }

    fn query_count(&self, sql: &str) -> i64 {
        scalar_i64(&sql_batches(&self.rt, self.ctx(), sql))
    }
}

/// Cold-tier guard: fresh connection + table open + provider
/// registration per instance (constructor = the timed open); the first
/// query then pays lance's real cold scan, exactly like the infino cold
/// guard pays its fresh-cache read path.
pub struct LanceSqlColdGuard<'a> {
    index: &'a LanceSqlIndex,
    ctx: SessionContext,
}

impl<'a> LanceSqlColdGuard<'a> {
    pub fn open(index: &'a LanceSqlIndex) -> Self {
        let ctx = index.rt.block_on(async {
            let table = open_sql_table(&index.location.uri, &index.location.storage_options).await;
            register_sql_ctx(&table).await
        });
        Self { index, ctx }
    }
}

impl SqlRead for LanceSqlColdGuard<'_> {
    fn query_rows(&self, sql: &str) -> usize {
        sql_batches(&self.index.rt, &self.ctx, sql)
            .iter()
            .map(|b| b.num_rows())
            .sum()
    }

    fn query_payload(&self, sql: &str) -> (u64, u64) {
        payload_bytes(&sql_batches(&self.index.rt, &self.ctx, sql))
    }

    fn query_count(&self, sql: &str) -> i64 {
        scalar_i64(&sql_batches(&self.index.rt, &self.ctx, sql))
    }
}

fn delete_index(index: LanceSqlIndex) {
    if matches!(index.location.storage, LanceStorage::Remote) {
        let uri = index.location.uri.clone();
        let storage_options = index.location.storage_options.clone();
        index.rt.block_on(async move {
            let db = connect(&uri, &storage_options).await;
            let _ = db.drop_table(SQL_TABLE, &[]).await;
        });
    }
}

impl SqlEngine for LanceSqlEngine {
    type Index = LanceSqlIndex;

    fn name() -> &'static str {
        "lancedb"
    }

    fn capabilities() -> Capabilities {
        Capabilities {
            fts: true,
            vector: true,
            sql: true,
            hybrid: true,
            ..Default::default()
        }
    }

    fn create() -> Self::Index {
        create_index(LanceLocation::local())
    }

    fn write(index: &mut Self::Index, rows: &[SqlRow<'_>]) {
        write_index(index, rows);
    }

    fn parallel_write(rows: &[SqlRow<'_>], writers: usize) {
        parallel_write_index(rows, writers, false);
    }

    fn read(index: &Self::Index, sql: &str) -> SqlOutput {
        read_index(index, sql)
    }

    fn close(index: &mut Self::Index) {
        index.ctx = None;
        index.table = None;
    }

    fn delete(index: Self::Index) {
        delete_index(index);
    }
}

impl SqlEngine for LanceS3SqlEngine {
    type Index = LanceSqlIndex;

    fn name() -> &'static str {
        lance_peer_label()
    }

    fn capabilities() -> Capabilities {
        LanceSqlEngine::capabilities()
    }

    fn create() -> Self::Index {
        create_index(LanceLocation::object_store("sql"))
    }

    fn write(index: &mut Self::Index, rows: &[SqlRow<'_>]) {
        write_index(index, rows);
    }

    fn parallel_write(rows: &[SqlRow<'_>], writers: usize) {
        parallel_write_index(rows, writers, true);
    }

    fn read(index: &Self::Index, sql: &str) -> SqlOutput {
        read_index(index, sql)
    }

    fn close(index: &mut Self::Index) {
        index.ctx = None;
        index.table = None;
    }

    fn delete(index: Self::Index) {
        delete_index(index);
    }
}
