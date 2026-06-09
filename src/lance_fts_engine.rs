// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! LanceDB peer implementation of [`FtsEngine`].
//!
//! Builds a Lance dataset with an inverted (BM25/FTS) index over the
//! corpus text column. Stemming, stop-word removal, and ASCII folding
//! are disabled to match the simple tokenizer the other engines use.

use std::sync::Arc;

use arrow_array::{RecordBatch, RecordBatchIterator, RecordBatchReader, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::index::Index;
use lancedb::index::scalar::{FtsIndexBuilder, FtsQuery, FullTextSearchQuery, MatchQuery, Operator};
use lancedb::{DistanceType, Table};
use tempfile::TempDir;
use tokio::runtime::Runtime;

use infino_bench_utils::harness::{BoolMode, Capabilities, FtsEngine, Hit};

const ID_COL: &str = "id";
const FTS_TABLE: &str = "fts";

/// Build one Lance FTS dataset at `uri` from `docs`, returning the opened
/// table. Shared by the queryable `write` and the build-throughput probe.
async fn build_fts_table(uri: &str, column: &str, docs: &[(u64, &str)]) -> Table {
    let schema = Arc::new(Schema::new(vec![
        Field::new(ID_COL, DataType::UInt64, false),
        Field::new(column, DataType::Utf8, false),
    ]));
    let ids = UInt64Array::from(docs.iter().map(|(id, _)| *id).collect::<Vec<_>>());
    let texts = StringArray::from(docs.iter().map(|(_, t)| *t).collect::<Vec<&str>>());
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(texts)])
        .expect("build RecordBatch");
    let reader: Box<dyn RecordBatchReader + Send> =
        Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));

    let db = lancedb::connect(uri).execute().await.expect("lancedb connect");
    let table = db
        .create_table(FTS_TABLE, reader)
        .execute()
        .await
        .expect("create lance fts table");
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

/// LanceDB as a comparison engine.
pub struct LanceFtsEngine;

/// Sealed Lance FTS index: the opened table, indexed column, and the
/// tokio runtime that drives the async LanceDB calls.
pub struct LanceFtsIndex {
    rt: Runtime,
    _dir: TempDir,
    uri: String,
    column: String,
    table: Option<Table>,
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
        let dir = tempfile::tempdir().expect("lance tempdir");
        let uri = dir.path().to_str().expect("utf8 temp path").to_string();
        LanceFtsIndex {
            rt: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime"),
            _dir: dir,
            uri,
            column: column.to_string(),
            table: None,
        }
    }

    fn write(index: &mut Self::Index, docs: &[(u64, &str)]) {
        let uri = index.uri.clone();
        let column = index.column.clone();
        let table = index.rt.block_on(build_fts_table(&uri, &column, docs));
        index.table = Some(table);
    }

    fn parallel_write(column: &str, docs: &[(u64, &str)], writers: usize) {
        if writers <= 1 {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let dir = tempfile::tempdir().expect("lance tempdir");
            let uri = dir.path().to_str().expect("utf8 temp path").to_string();
            let table = rt.block_on(build_fts_table(&uri, column, docs));
            std::hint::black_box(&table);
            return;
        }
        // Parallel build: shard the corpus across `writers` builders,
        // each emitting its own Lance dataset. Build-only — tables discarded.
        let shard_len = docs.len().div_ceil(writers);
        let shards: Vec<Table> = docs
            .chunks(shard_len)
            .map(|shard| {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                let dir = tempfile::tempdir().expect("lance tempdir");
                let uri = dir.path().to_str().expect("utf8 temp path").to_string();
                rt.block_on(build_fts_table(&uri, column, shard))
            })
            .collect();
        std::hint::black_box(shards);
    }

    fn read(index: &Self::Index, terms: &[&str], k: usize, mode: BoolMode) -> Vec<Hit> {
        let table = index.table.as_ref().expect("read before write");
        let operator = match mode {
            BoolMode::Or => Operator::Or,
            BoolMode::And => Operator::And,
        };
        let match_q = MatchQuery::new(terms.join(" "))
            .with_column(Some(index.column.clone()))
            .with_operator(operator);
        let fts_query = FullTextSearchQuery::new_query(FtsQuery::Match(match_q));

        index.rt.block_on(async {
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
                    .downcast_ref::<arrow_array::Float32Array>()
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

    fn close(index: &mut Self::Index) {
        index.table = None;
    }

    fn delete(_index: Self::Index) {
        // Dropping the index drops the table handle and the TempDir.
    }
}

#[cfg(test)]
mod tests {
    use super::{BoolMode, FtsEngine, LanceFtsEngine};

    #[test]
    fn open_write_read_roundtrip() {
        let mut idx = LanceFtsEngine::create("title");
        let docs: [(u64, &str); 3] = [
            (0, "the quick brown fox"),
            (1, "a lazy sleeping dog"),
            (2, "quick foxes leap"),
        ];
        LanceFtsEngine::write(&mut idx, &docs);

        let hits = LanceFtsEngine::read(&idx, &["quick"], 10, BoolMode::Or);
        let ids: Vec<u64> = hits.iter().map(|h| h.doc_id).collect();
        assert!(
            ids.contains(&0) && ids.contains(&2),
            "docs 0 and 2 contain 'quick'; got {ids:?}"
        );
        assert!(!ids.contains(&1), "doc 1 has no 'quick'; got {ids:?}");

        let and_hits = LanceFtsEngine::read(&idx, &["quick", "fox"], 10, BoolMode::And);
        let and_ids: Vec<u64> = and_hits.iter().map(|h| h.doc_id).collect();
        assert!(
            and_ids.contains(&0),
            "doc 0 has both 'quick' and 'fox'; got {and_ids:?}"
        );
    }
}
