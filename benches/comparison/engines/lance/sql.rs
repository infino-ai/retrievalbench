// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! LanceDB peer implementation of [`SqlEngine`].
//!
//! Builds a Lance dataset with the portable scalar columns the SQL
//! battery references (`title`, `category`, `rating`) and answers queries
//! through a DataFusion context backed by the Lance dataset.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use arrow_array::{Int64Array, RecordBatch, RecordBatchIterator, RecordBatchReader, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use datafusion::datasource::MemTable;
use datafusion::prelude::SessionContext;
use futures::TryStreamExt;
use lancedb::query::ExecutableQuery;
use lancedb::Table;
use tempfile::TempDir;
use tokio::runtime::Runtime;

use infino_bench_utils::harness::{Capabilities, SqlEngine, SqlOutput, SqlRow};

const SQL_TABLE: &str = "sql";
const SQL_VIEW: &str = "supertable";

enum LanceStorage {
    Local { _dir: TempDir },
    S3,
}

struct LanceLocation {
    uri: String,
    storage_options: Vec<(String, String)>,
    storage: LanceStorage,
}

impl LanceLocation {
    fn local() -> Self {
        let dir = tempfile::tempdir().expect("lance tempdir");
        let uri = dir.path().to_str().expect("utf8 temp path").to_string();
        Self {
            uri,
            storage_options: Vec::new(),
            storage: LanceStorage::Local { _dir: dir },
        }
    }

    fn s3(prefix: &str) -> Self {
        let bucket = infino_bench_utils::tiers::real_s3_bucket_env()
            .expect("INFINO_REAL_S3_BUCKET required for LanceDB S3 tier");
        let root = infino_bench_utils::tiers::real_s3_prefix_root("retrievalbench-lance");
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_nanos();
        let uri = format!(
            "s3://{}/{}/{prefix}-{}-{unique}",
            bucket,
            root.trim_matches('/'),
            std::process::id(),
        );
        let mut storage_options = Vec::new();
        if let Ok(region) = std::env::var("AWS_REGION").or_else(|_| std::env::var("AWS_DEFAULT_REGION")) {
            storage_options.push(("aws_region".to_string(), region));
        }
        Self {
            uri,
            storage_options,
            storage: LanceStorage::S3,
        }
    }
}

fn new_runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

async fn connect(uri: &str, storage_options: &[(String, String)]) -> lancedb::Connection {
    lancedb::connect(uri)
        .storage_options(storage_options.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .execute()
        .await
        .expect("lancedb connect")
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
    let ids = UInt64Array::from(rows.iter().map(|r| r.doc_id).collect::<Vec<_>>());
    let titles = StringArray::from(rows.iter().map(|r| r.title).collect::<Vec<&str>>());
    let categories = StringArray::from(rows.iter().map(|r| r.category).collect::<Vec<&str>>());
    let ratings = Int64Array::from(rows.iter().map(|r| r.score).collect::<Vec<_>>())
;
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
    let reader: Box<dyn RecordBatchReader + Send> =
        Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));

    let db = connect(uri, storage_options).await;
    db.create_table(SQL_TABLE, reader)
        .execute()
        .await
        .expect("create lance sql table")
}

async fn register_sql_ctx(table: &Table) -> SessionContext {
    let ctx = SessionContext::new();
    let stream = table.query().execute().await.expect("scan lance table");
    let batches: Vec<RecordBatch> = stream.try_collect().await.expect("collect lance scan");
    let schema = batches
        .first()
        .map(|b| b.schema())
        .unwrap_or_else(|| Arc::new(Schema::empty()));
    let mem = MemTable::try_new(schema, vec![batches]).expect("build memtable");
    ctx.register_table(SQL_VIEW, Arc::new(mem))
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

fn parallel_write_index(rows: &[SqlRow<'_>], writers: usize, s3: bool) {
    if writers <= 1 {
        let rt = new_runtime();
        let location = if s3 {
            LanceLocation::s3("sql")
        } else {
            LanceLocation::local()
        };
        let table = rt.block_on(build_sql_table(&location.uri, &location.storage_options, rows));
        std::hint::black_box(&table);
        return;
    }
    let rows_per = rows.len().div_ceil(writers);
    let built: Vec<Table> = rows
        .chunks(rows_per)
        .enumerate()
        .map(|(i, shard)| {
            let rt = new_runtime();
            let location = if s3 {
                LanceLocation::s3(&format!("sql-shard-{i}"))
            } else {
                LanceLocation::local()
            };
            rt.block_on(build_sql_table(&location.uri, &location.storage_options, shard))
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

fn delete_index(index: LanceSqlIndex) {
    if matches!(index.location.storage, LanceStorage::S3) {
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
        "lancedb-s3"
    }

    fn capabilities() -> Capabilities {
        LanceSqlEngine::capabilities()
    }

    fn create() -> Self::Index {
        create_index(LanceLocation::s3("sql"))
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
