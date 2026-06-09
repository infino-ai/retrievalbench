// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! LanceDB peer implementation of [`SqlEngine`].
//!
//! Builds a Lance dataset with the portable scalar columns the SQL
//! battery references (`title`, `category`, `rating`) and answers queries
//! through a DataFusion context backed by the Lance dataset.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, RecordBatchIterator, RecordBatchReader, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use datafusion::datasource::MemTable;
use datafusion::prelude::SessionContext;
use futures::TryStreamExt;
use lancedb::Table;
use tempfile::TempDir;
use tokio::runtime::Runtime;

use infino_bench_utils::harness::{Capabilities, SqlEngine, SqlOutput, SqlRow};

const SQL_TABLE: &str = "sql";
const SQL_VIEW: &str = "supertable";

/// Build a Lance dataset at `uri` from `rows`, returning the opened table.
async fn build_sql_table(uri: &str, rows: &[SqlRow<'_>]) -> Table {
    let schema = Arc::new(Schema::new(vec![
        Field::new("doc_id", DataType::UInt64, false),
        Field::new("title", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("rating", DataType::Int64, false),
    ]));
    let ids = UInt64Array::from(rows.iter().map(|r| r.doc_id).collect::<Vec<_>>());
    let titles = StringArray::from(rows.iter().map(|r| r.title).collect::<Vec<&str>>());
    let categories = StringArray::from(rows.iter().map(|r| r.category).collect::<Vec<&str>>());
    let ratings = Int64Array::from(rows.iter().map(|r| r.score).collect::<Vec<_>>());
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

    let db = lancedb::connect(uri).execute().await.expect("lancedb connect");
    db.create_table(SQL_TABLE, reader)
        .execute()
        .await
        .expect("create lance sql table")
}

/// Build a DataFusion context that answers `FROM supertable` against the
/// Lance dataset. The dataset is scanned and registered as an in-memory
/// `MemTable` mirroring Infino's in-memory supertable query model.
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

/// LanceDB as a SQL comparison engine.
pub struct LanceSqlEngine;

/// Sealed Lance SQL index: runtime, backing temp dir, the opened table,
/// and a DataFusion context with the dataset registered.
pub struct LanceSqlIndex {
    rt: Runtime,
    _dir: TempDir,
    uri: String,
    table: Option<Table>,
    ctx: Option<SessionContext>,
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
        let dir = tempfile::tempdir().expect("lance tempdir");
        let uri = dir.path().to_str().expect("utf8 temp path").to_string();
        LanceSqlIndex {
            rt: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime"),
            _dir: dir,
            uri,
            table: None,
            ctx: None,
        }
    }

    fn write(index: &mut Self::Index, rows: &[SqlRow<'_>]) {
        let uri = index.uri.clone();
        let (table, ctx) = index.rt.block_on(async {
            let table = build_sql_table(&uri, rows).await;
            let ctx = register_sql_ctx(&table).await;
            (table, ctx)
        });
        index.table = Some(table);
        index.ctx = Some(ctx);
    }

    fn parallel_write(rows: &[SqlRow<'_>], writers: usize) {
        if writers <= 1 {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let dir = tempfile::tempdir().expect("lance tempdir");
            let uri = dir.path().to_str().expect("utf8 temp path").to_string();
            let table = rt.block_on(build_sql_table(&uri, rows));
            std::hint::black_box(&table);
            return;
        }
        // Parallel build: shard the rows across `writers` builders,
        // each emitting its own Lance dataset. Build-only — tables discarded.
        let rows_per = rows.len().div_ceil(writers);
        let built: Vec<Table> = rows
            .chunks(rows_per)
            .map(|shard| {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                let dir = tempfile::tempdir().expect("lance tempdir");
                let uri = dir.path().to_str().expect("utf8 temp path").to_string();
                rt.block_on(build_sql_table(&uri, shard))
            })
            .collect();
        std::hint::black_box(built);
    }

    fn read(index: &Self::Index, sql: &str) -> SqlOutput {
        let ctx = index.ctx.as_ref().expect("read before write");
        let rows = index.rt.block_on(async {
            let df = ctx.sql(sql).await.expect("plan sql");
            let batches = df.collect().await.expect("collect sql result");
            batches.iter().map(|b| b.num_rows()).sum::<usize>()
        });
        SqlOutput { rows }
    }

    fn close(index: &mut Self::Index) {
        index.ctx = None;
        index.table = None;
    }

    fn delete(_index: Self::Index) {
        // Dropping the index drops the context, table handle, and TempDir.
    }
}

#[cfg(test)]
mod tests {
    use super::{LanceSqlEngine, SqlEngine};
    use infino_bench_utils::harness::SqlRow;

    #[test]
    fn sql_scalar_roundtrip() {
        let mut idx = LanceSqlEngine::create();
        let rows = [
            SqlRow {
                doc_id: 0,
                title: "rust async runtime",
                category: "rust",
                score: 10,
            },
            SqlRow {
                doc_id: 1,
                title: "python data pipeline",
                category: "python",
                score: 20,
            },
            SqlRow {
                doc_id: 2,
                title: "rust web framework",
                category: "rust",
                score: 30,
            },
        ];
        LanceSqlEngine::write(&mut idx, &rows);

        let total = LanceSqlEngine::read(&idx, "SELECT * FROM supertable");
        assert_eq!(total.rows, 3, "all rows visible; got {}", total.rows);

        let rust = LanceSqlEngine::read(
            &idx,
            "SELECT COUNT(*) AS n FROM supertable WHERE category = 'rust'",
        );
        assert_eq!(rust.rows, 1, "count is one row; got {}", rust.rows);

        let groups = LanceSqlEngine::read(
            &idx,
            "SELECT category, COUNT(*) AS n FROM supertable GROUP BY category",
        );
        assert_eq!(groups.rows, 2, "two category groups; got {}", groups.rows);
    }
}
