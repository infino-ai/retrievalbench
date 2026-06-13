// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! LanceDB peer implementation of [`FtsEngine`].
//!
//! Builds a Lance dataset with an inverted (BM25/FTS) index over the
//! corpus text column. Stemming, stop-word removal, and ASCII folding
//! are disabled to match the simple tokenizer the other engines use.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use arrow_array::{Float32Array, RecordBatch, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::Table;
use lancedb::index::Index;
use lancedb::index::scalar::{
    FtsIndexBuilder, FtsQuery, FullTextSearchQuery, MatchQuery, Operator,
};
use lancedb::query::{ExecutableQuery, QueryBase};
use tempfile::TempDir;
use tokio::runtime::Runtime;

use infino_bench_utils::harness::{BoolMode, Capabilities, FtsEngine, Hit};

const ID_COL: &str = "id";
const FTS_TABLE: &str = "fts";
const LANCE_TEXT_BATCH_ROWS: usize = 100_000;

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
        if let Ok(region) =
            std::env::var("AWS_REGION").or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
        {
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
        .storage_options(
            storage_options
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        )
        .execute()
        .await
        .expect("lancedb connect")
}

async fn open_fts_table(uri: &str, storage_options: &[(String, String)]) -> Table {
    connect(uri, storage_options)
        .await
        .open_table(FTS_TABLE)
        .execute()
        .await
        .expect("open lance fts table")
}

async fn build_fts_table(
    uri: &str,
    storage_options: &[(String, String)],
    column: &str,
    docs: &[(u64, &str)],
) -> Table {
    let schema = Arc::new(Schema::new(vec![
        Field::new(ID_COL, DataType::UInt64, false),
        Field::new(column, DataType::Utf8, false),
    ]));
    let db = connect(uri, storage_options).await;
    let table = db
        .create_empty_table(FTS_TABLE, schema.clone())
        .execute()
        .await
        .expect("create lance fts table");
    for chunk in docs.chunks(LANCE_TEXT_BATCH_ROWS) {
        let ids = UInt64Array::from(chunk.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        let texts = StringArray::from(chunk.iter().map(|(_, t)| *t).collect::<Vec<&str>>());
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(texts)])
            .expect("build RecordBatch");
        table
            .add(vec![batch])
            .execute()
            .await
            .expect("add lance fts batch");
    }
    let params = FtsIndexBuilder::default()
        .stem(false)
        .remove_stop_words(false)
        .ascii_folding(false);
    table
        .create_index(&[column], Index::FTS(params))
        .execute()
        .await
        .expect("create FTS index");
    table
}

pub struct LanceFtsEngine;
pub struct LanceS3FtsEngine;

pub struct LanceFtsIndex {
    rt: Runtime,
    location: LanceLocation,
    column: String,
    table: Option<Table>,
}

impl LanceFtsIndex {
    /// Table opened on the measured 1-writer artifact.
    pub fn table(&self) -> &Table {
        self.table.as_ref().expect("table requested before write")
    }
}

fn create_index(column: &str, location: LanceLocation) -> LanceFtsIndex {
    LanceFtsIndex {
        rt: new_runtime(),
        location,
        column: column.to_string(),
        table: None,
    }
}

fn write_index(index: &mut LanceFtsIndex, docs: &[(u64, &str)]) {
    let uri = index.location.uri.clone();
    let storage_options = index.location.storage_options.clone();
    let column = index.column.clone();
    let table = index
        .rt
        .block_on(build_fts_table(&uri, &storage_options, &column, docs));
    index.table = Some(table);
}

fn parallel_write_index(column: &str, docs: &[(u64, &str)], writers: usize, s3: bool) {
    if writers <= 1 {
        let rt = new_runtime();
        let location = if s3 {
            LanceLocation::s3("fts")
        } else {
            LanceLocation::local()
        };
        let table = rt.block_on(build_fts_table(
            &location.uri,
            &location.storage_options,
            column,
            docs,
        ));
        std::hint::black_box(&table);
        return;
    }
    // Concurrent shard builds — the same independent-shard semantics as
    // Infino's `par_chunks` parallel build. Build-only — tables discarded.
    let shard_len = docs.len().div_ceil(writers);
    std::thread::scope(|scope| {
        let handles: Vec<_> = docs
            .chunks(shard_len)
            .enumerate()
            .map(|(i, shard)| {
                scope.spawn(move || {
                    let rt = new_runtime();
                    let location = if s3 {
                        LanceLocation::s3(&format!("fts-shard-{i}"))
                    } else {
                        LanceLocation::local()
                    };
                    let table = rt.block_on(build_fts_table(
                        &location.uri,
                        &location.storage_options,
                        column,
                        shard,
                    ));
                    std::hint::black_box(&table);
                    drop(table);
                })
            })
            .collect();
        for h in handles {
            h.join().expect("lance fts shard build thread panicked");
        }
    });
}

fn read_table(
    rt: &Runtime,
    table: &Table,
    column: &str,
    terms: &[&str],
    k: usize,
    mode: BoolMode,
) -> Vec<Hit> {
    let operator = match mode {
        BoolMode::Or => Operator::Or,
        BoolMode::And => Operator::And,
    };
    let match_q = MatchQuery::new(terms.join(" "))
        .with_column(Some(column.to_string()))
        .with_operator(operator);
    let fts_query = FullTextSearchQuery::new_query(FtsQuery::Match(match_q));

    rt.block_on(async {
        let stream = table
            .query()
            .full_text_search(fts_query)
            .limit(k.max(1))
            .execute()
            .await
            .expect("fts query execute");
        let batches: Vec<RecordBatch> = stream.try_collect().await.expect("collect stream");

        let mut out = Vec::with_capacity(k);
        for b in &batches {
            let ids = b
                .column_by_name(ID_COL)
                .expect("id column")
                .as_any()
                .downcast_ref::<UInt64Array>()
                .expect("u64 id column");
            let scores = b
                .column_by_name("_score")
                .expect("_score column")
                .as_any()
                .downcast_ref::<Float32Array>()
                .expect("f32 _score column");
            for i in 0..b.num_rows() {
                out.push(Hit {
                    doc_id: ids.value(i),
                    score: scores.value(i),
                });
            }
        }
        out
    })
}

fn read_index(index: &LanceFtsIndex, terms: &[&str], k: usize, mode: BoolMode) -> Vec<Hit> {
    read_table(&index.rt, index.table(), &index.column, terms, k, mode)
}

impl LanceFtsIndex {
    /// Reopen the same S3 artifact and run one FTS query. Used by the
    /// comparison cold tier so cold does not include rebuild time.
    pub fn cold_read(&self, terms: &[&str], k: usize, mode: BoolMode) -> Vec<Hit> {
        let uri = self.location.uri.clone();
        let storage_options = self.location.storage_options.clone();
        let table = self.rt.block_on(open_fts_table(&uri, &storage_options));
        read_table(&self.rt, &table, &self.column, terms, k, mode)
    }
}

fn delete_index(index: LanceFtsIndex) {
    if matches!(index.location.storage, LanceStorage::S3) {
        let uri = index.location.uri.clone();
        let storage_options = index.location.storage_options.clone();
        index.rt.block_on(async move {
            let db = connect(&uri, &storage_options).await;
            let _ = db.drop_table(FTS_TABLE, &[]).await;
        });
    }
}

impl FtsEngine for LanceFtsEngine {
    type Index = LanceFtsIndex;

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

    fn create(column: &str) -> Self::Index {
        create_index(column, LanceLocation::local())
    }

    fn write(index: &mut Self::Index, docs: &[(u64, &str)]) {
        write_index(index, docs);
    }

    fn parallel_write(column: &str, docs: &[(u64, &str)], writers: usize) {
        parallel_write_index(column, docs, writers, false);
    }

    fn read(index: &Self::Index, terms: &[&str], k: usize, mode: BoolMode) -> Vec<Hit> {
        read_index(index, terms, k, mode)
    }

    fn close(index: &mut Self::Index) {
        index.table = None;
    }

    fn delete(index: Self::Index) {
        delete_index(index);
    }
}

impl FtsEngine for LanceS3FtsEngine {
    type Index = LanceFtsIndex;

    fn name() -> &'static str {
        "lancedb-s3"
    }

    fn capabilities() -> Capabilities {
        LanceFtsEngine::capabilities()
    }

    fn create(column: &str) -> Self::Index {
        create_index(column, LanceLocation::s3("fts"))
    }

    fn write(index: &mut Self::Index, docs: &[(u64, &str)]) {
        write_index(index, docs);
    }

    fn parallel_write(column: &str, docs: &[(u64, &str)], writers: usize) {
        parallel_write_index(column, docs, writers, true);
    }

    fn read(index: &Self::Index, terms: &[&str], k: usize, mode: BoolMode) -> Vec<Hit> {
        read_index(index, terms, k, mode)
    }

    fn close(index: &mut Self::Index) {
        index.table = None;
    }

    fn delete(index: Self::Index) {
        delete_index(index);
    }
}
