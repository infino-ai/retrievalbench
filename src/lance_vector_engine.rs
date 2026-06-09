// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! LanceDB peer implementation of [`VectorEngine`].
//!
//! Builds a Lance dataset with an `IVF_PQ` index and queries it via
//! `nearest_to`. Vector parity with Infino: `num_partitions = n_cent`,
//! searched with `nprobes` / `refine_factor`.

use std::sync::Arc;

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator,
    RecordBatchReader, UInt64Array,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::index::Index;
use lancedb::index::vector::IvfPqIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{DistanceType, Table};
use tempfile::TempDir;
use tokio::runtime::Runtime;

use infino_bench_utils::corpus;
use infino_bench_utils::harness::{
    Capabilities, VectorEngine, VectorHit, VectorMetric, VectorSearch,
};

const ID_COL: &str = "id";
const VEC_COL: &str = "vector";
const VEC_TABLE: &str = "vectors";

fn map_metric(metric: VectorMetric) -> DistanceType {
    match metric {
        VectorMetric::L2Sq => DistanceType::L2,
        VectorMetric::Cosine => DistanceType::Cosine,
        VectorMetric::NegDot => DistanceType::Dot,
    }
}

/// Build one Lance vector dataset at `uri` from `vectors`, returning the
/// opened table. Shared by the queryable `write` and the build-throughput
/// probe.
async fn build_vector_table(
    uri: &str,
    vectors: &[f32],
    dim: usize,
    metric: VectorMetric,
    num_partitions: u32,
) -> Table {
    let n_docs = vectors.len() / dim;
    let item = Arc::new(Field::new("item", DataType::Float32, true));
    let schema = Arc::new(Schema::new(vec![
        Field::new(ID_COL, DataType::UInt64, false),
        Field::new(
            VEC_COL,
            DataType::FixedSizeList(item.clone(), dim as i32),
            false,
        ),
    ]));

    let ids = UInt64Array::from((0..n_docs as u64).collect::<Vec<_>>());
    let flat = Float32Array::from(vectors.to_vec());
    let fsl = FixedSizeListArray::try_new(item, dim as i32, Arc::new(flat), None)
        .expect("build FixedSizeListArray");
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(fsl)])
        .expect("build RecordBatch");
    let reader: Box<dyn RecordBatchReader + Send> =
        Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));

    let db = lancedb::connect(uri).execute().await.expect("lancedb connect");
    let table = db
        .create_table(VEC_TABLE, reader)
        .execute()
        .await
        .expect("create lance table");
    table
        .create_index(
            &[VEC_COL],
            Index::IvfPq(
                IvfPqIndexBuilder::default()
                    .num_partitions(num_partitions)
                    .distance_type(map_metric(metric)),
            ),
        )
        .execute()
        .await
        .expect("create IVF-PQ index");
    table
}

/// LanceDB as a vector comparison engine.
pub struct LanceVectorEngine;

/// Sealed Lance vector index: the opened table and the tokio runtime.
pub struct LanceVectorIndex {
    rt: Runtime,
    _dir: TempDir,
    uri: String,
    dim: usize,
    metric: VectorMetric,
    n_cent: usize,
    table: Option<Table>,
}

impl VectorEngine for LanceVectorEngine {
    type Index = LanceVectorIndex;

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

    fn create(_column: &str, dim: usize, metric: VectorMetric, n_cent: usize) -> Self::Index {
        let dir = tempfile::tempdir().expect("lance tempdir");
        let uri = dir.path().to_str().expect("utf8 temp path").to_string();
        LanceVectorIndex {
            rt: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime"),
            _dir: dir,
            uri,
            dim,
            metric,
            n_cent,
            table: None,
        }
    }

    fn write(index: &mut Self::Index, vectors: &[f32]) {
        let n_docs = vectors.len() / index.dim;
        let num_partitions = index.n_cent.max(1).min(n_docs.max(1)) as u32;
        let uri = index.uri.clone();
        let (dim, metric) = (index.dim, index.metric);
        let table = index
            .rt
            .block_on(build_vector_table(&uri, vectors, dim, metric, num_partitions));
        index.table = Some(table);
    }

    fn parallel_write(
        _column: &str,
        vectors: &[f32],
        dim: usize,
        metric: VectorMetric,
        writers: usize,
    ) {
        if writers <= 1 {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let dir = tempfile::tempdir().expect("lance tempdir");
            let uri = dir.path().to_str().expect("utf8 temp path").to_string();
            let n_docs = vectors.len() / dim;
            let num_partitions = corpus::n_cent(n_docs).max(1).min(n_docs.max(1)) as u32;
            let table = rt.block_on(build_vector_table(&uri, vectors, dim, metric, num_partitions));
            std::hint::black_box(&table);
            return;
        }
        // Parallel build: shard the corpus across `writers` builders,
        // each emitting its own Lance dataset. Build-only — tables discarded.
        let n_docs = vectors.len() / dim;
        let docs_per_shard = n_docs.div_ceil(writers);
        let shards: Vec<Table> = (0..writers)
            .filter_map(|shard| {
                let start_doc = shard * docs_per_shard;
                if start_doc >= n_docs {
                    return None;
                }
                let len_docs = docs_per_shard.min(n_docs - start_doc);
                let start = start_doc * dim;
                let end = (start_doc + len_docs) * dim;
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                let dir = tempfile::tempdir().expect("lance tempdir");
                let uri = dir.path().to_str().expect("utf8 temp path").to_string();
                let num_partitions = corpus::n_cent(len_docs).max(1).min(len_docs.max(1)) as u32;
                Some(rt.block_on(build_vector_table(
                    &uri,
                    &vectors[start..end],
                    dim,
                    metric,
                    num_partitions,
                )))
            })
            .collect();
        std::hint::black_box(shards);
    }

    fn read(index: &Self::Index, query: &[f32], k: usize, search: VectorSearch) -> Vec<VectorHit> {
        let table = index.table.as_ref().expect("read before write");
        index.rt.block_on(async {
            let mut q = table
                .query()
                .nearest_to(query.to_vec())
                .expect("nearest_to")
                .limit(k.max(1))
                .nprobes(search.nprobe.max(1));
            if search.rerank_mult > 1 {
                q = q.refine_factor(search.rerank_mult as u32);
            }
            let stream = q.execute().await.expect("vector query execute");
            let batches: Vec<RecordBatch> = stream.try_collect().await.expect("collect stream");

            let mut out = Vec::with_capacity(k);
            for b in &batches {
                let ids = b
                    .column_by_name(ID_COL)
                    .expect("id column")
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .expect("u64 id column");
                let dist = b
                    .column_by_name("_distance")
                    .expect("_distance column")
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .expect("f32 _distance column");
                for i in 0..b.num_rows() {
                    out.push(VectorHit {
                        doc_id: ids.value(i),
                        distance: dist.value(i),
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
    use super::{LanceVectorEngine, VectorEngine, VectorMetric, VectorSearch};

    #[test]
    fn open_write_read_roundtrip() {
        let dim = 32usize;
        let n = 1024usize;
        let mut vectors = vec![0.0f32; n * dim];
        for (i, chunk) in vectors.chunks_mut(dim).enumerate() {
            for (j, v) in chunk.iter_mut().enumerate() {
                *v = ((i + j) % 17) as f32;
            }
        }

        let mut idx = LanceVectorEngine::create("v", dim, VectorMetric::L2Sq, 8);
        LanceVectorEngine::write(&mut idx, &vectors);

        let query = vectors[5 * dim..6 * dim].to_vec();
        let hits = LanceVectorEngine::read(
            &idx,
            &query,
            10,
            VectorSearch {
                nprobe: 8,
                rerank_mult: 64,
            },
        );
        let ids: Vec<u64> = hits.iter().map(|h| h.doc_id).collect();
        assert!(ids.contains(&5), "exact-match query should return doc 5; got {ids:?}");
    }
}
