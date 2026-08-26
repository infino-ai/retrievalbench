// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! LanceDB peer implementation of [`VectorEngine`].
//!
//! Builds a Lance dataset with an `IVF_PQ` index and queries it via
//! `nearest_to`. The index is built at LanceDB's own defaults (it
//! auto-sizes `num_partitions`); only the distance metric is set.

use std::sync::Arc;

use arrow_array::{Array, FixedSizeListArray, Float32Array, RecordBatch, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::index::Index;
use lancedb::index::vector::IvfPqIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use lancedb::{DistanceType, Table};
use tokio::runtime::Runtime;

use super::location::{LanceLocation, LanceStorage, lance_peer_label};

use infino_bench_utils::executors::vector::{ENGINE_DEFAULT, VectorRead};
use infino_bench_utils::harness::{
    Capabilities, VectorEngine, VectorHit, VectorMetric, VectorSearch,
};

const ID_COL: &str = "id";
const VEC_COL: &str = "vector";
const VEC_TABLE: &str = "vectors";
/// Rows per appended Arrow batch. One batch covers the whole 1M-doc
/// superfile tier (identical to the original single-batch build); larger
/// corpora append in chunks so the build never materializes the full
/// fp32 corpus a second time (10M × dim=1024 would be a 41 GB copy).
const LANCE_VEC_BATCH_ROWS: usize = 1_000_000;

fn map_metric(metric: VectorMetric) -> DistanceType {
    match metric {
        VectorMetric::L2Sq => DistanceType::L2,
        VectorMetric::Cosine => DistanceType::Cosine,
        VectorMetric::NegDot => DistanceType::Dot,
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

async fn open_vector_table(uri: &str, storage_options: &[(String, String)]) -> Table {
    connect(uri, storage_options)
        .await
        .open_table(VEC_TABLE)
        .execute()
        .await
        .expect("open lance vector table")
}

async fn build_table(
    uri: &str,
    storage_options: &[(String, String)],
    vectors: &[f32],
    dim: usize,
    metric: VectorMetric,
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

    let db = connect(uri, storage_options).await;
    let table = db
        .create_empty_table(VEC_TABLE, schema.clone())
        .execute()
        .await
        .expect("create lance table");
    for start in (0..n_docs).step_by(LANCE_VEC_BATCH_ROWS) {
        let len = LANCE_VEC_BATCH_ROWS.min(n_docs - start);
        let ids = UInt64Array::from((start as u64..(start + len) as u64).collect::<Vec<_>>());
        let flat = Float32Array::from(vectors[start * dim..(start + len) * dim].to_vec());
        let fsl = FixedSizeListArray::try_new(item.clone(), dim as i32, Arc::new(flat), None)
            .expect("build FixedSizeListArray");
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(fsl)])
            .expect("build RecordBatch");
        table
            .add(vec![batch])
            .execute()
            .await
            .expect("add lance vector batch");
    }
    table
        .create_index(
            &[VEC_COL],
            Index::IvfPq(IvfPqIndexBuilder::default().distance_type(map_metric(metric))),
        )
        .execute()
        .await
        .expect("create IVF-PQ index");
    table
}

pub struct LanceVectorEngine;
pub struct LanceS3VectorEngine;

pub struct LanceVectorIndex {
    rt: Runtime,
    location: LanceLocation,
    #[allow(dead_code)]
    column: String,
    dim: usize,
    metric: VectorMetric,
    table: Option<Table>,
}

impl LanceVectorIndex {
    /// Table opened on the measured 1-writer artifact.
    pub fn table(&self) -> &Table {
        self.table.as_ref().expect("table requested before write")
    }
}

/// Shared recall-calibration hook: lets the engine-generic
/// `executors::vector::calibrate` grid drive Lance with the same
/// `(probe, refine)` vocabulary it uses for Infino. Lance ids are the
/// row's insertion position (`0..n_docs`), so they are already global.
impl VectorRead for LanceVectorIndex {
    fn topk_global(
        &self,
        _column: &str,
        query: &[f32],
        k: usize,
        nprobe: usize,
        rerank: usize,
    ) -> Vec<(u32, f32)> {
        read_index(
            self,
            query,
            k,
            VectorSearch {
                nprobe,
                rerank_mult: rerank,
            },
        )
        .into_iter()
        .map(|h| (h.doc_id as u32, h.distance))
        .collect()
    }

    fn search_params(&self, nprobe: usize, rerank: usize) -> String {
        if nprobe == ENGINE_DEFAULT && rerank == ENGINE_DEFAULT {
            "engine defaults".into()
        } else {
            format!("p={nprobe}, r={rerank}")
        }
    }
}

fn create_index(
    column: &str,
    dim: usize,
    metric: VectorMetric,
    location: LanceLocation,
) -> LanceVectorIndex {
    LanceVectorIndex {
        rt: new_runtime(),
        location,
        column: column.to_string(),
        dim,
        metric,
        table: None,
    }
}

fn write_index(index: &mut LanceVectorIndex, vectors: &[f32]) {
    let uri = index.location.uri.clone();
    let storage_options = index.location.storage_options.clone();
    let (dim, metric) = (index.dim, index.metric);
    let table = index.rt.block_on(build_table(
        &uri,
        &storage_options,
        vectors,
        dim,
        metric,
    ));
    index.table = Some(table);
}

fn parallel_write_index(
    _column: &str,
    vectors: &[f32],
    dim: usize,
    metric: VectorMetric,
    writers: usize,
    remote: bool,
) {
    if writers <= 1 {
        let rt = new_runtime();
        let location = if remote {
            LanceLocation::object_store("vector")
        } else {
            LanceLocation::local()
        };
        let table = rt.block_on(build_table(
            &location.uri,
            &location.storage_options,
            vectors,
            dim,
            metric,
        ));
        std::hint::black_box(&table);
        return;
    }
    // Concurrent shard builds — the same independent-shard semantics as
    // Infino's `par_chunks` parallel build. Build-only — tables discarded.
    let n_docs = vectors.len() / dim;
    let docs_per_shard = n_docs.div_ceil(writers);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..writers)
            .filter_map(|shard| {
                let start_doc = shard * docs_per_shard;
                if start_doc >= n_docs {
                    return None;
                }
                let len_docs = docs_per_shard.min(n_docs - start_doc);
                let slice = &vectors[start_doc * dim..(start_doc + len_docs) * dim];
                Some(scope.spawn(move || {
                    let rt = new_runtime();
                    let location = if remote {
                        LanceLocation::object_store(&format!("vector-shard-{shard}"))
                    } else {
                        LanceLocation::local()
                    };
                    let table = rt.block_on(build_table(
                        &location.uri,
                        &location.storage_options,
                        slice,
                        dim,
                        metric,
                    ));
                    std::hint::black_box(&table);
                    drop(table);
                }))
            })
            .collect();
        for h in handles {
            h.join().expect("lance vector shard build thread panicked");
        }
    });
}

fn read_table(
    rt: &Runtime,
    table: &Table,
    query: &[f32],
    k: usize,
    search: VectorSearch,
) -> Vec<VectorHit> {
    rt.block_on(async {
        // Project the id column only (`_distance` is system-added):
        // infino's measured path returns `_id` + score without decoding
        // row data, so the peer must not be billed for materializing its
        // 4 KB vector per hit either.
        let mut q = table
            .query()
            .nearest_to(query.to_vec())
            .expect("nearest_to")
            .select(Select::Columns(vec![ID_COL.to_string()]))
            .limit(k.max(1));
        // ENGINE_DEFAULT leaves LanceDB's own shipped search defaults in
        // place — the peer's `default` row must measure its serving
        // defaults, not a harness constant (mirror of infino's law-served
        // default path, where absent options select the default routing).
        if search.nprobe != ENGINE_DEFAULT {
            q = q.nprobes(search.nprobe);
        }
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

fn read_index(
    index: &LanceVectorIndex,
    query: &[f32],
    k: usize,
    search: VectorSearch,
) -> Vec<VectorHit> {
    read_table(&index.rt, index.table(), query, k, search)
}

impl LanceVectorIndex {
    /// Reopen the same object-store artifact and run one vector query. Used by the
    /// comparison cold tier so cold does not include rebuild time.
    pub fn cold_read(&self, query: &[f32], k: usize, search: VectorSearch) -> Vec<VectorHit> {
        let uri = self.location.uri.clone();
        let storage_options = self.location.storage_options.clone();
        let table = self.rt.block_on(open_vector_table(&uri, &storage_options));
        read_table(&self.rt, &table, query, k, search)
    }

    /// Open the cold object-store artifact without querying. The cold tier times only
    /// the search, so the open is excluded — matching the Infino cold path.
    pub fn cold_open(&self) -> Table {
        let uri = self.location.uri.clone();
        let storage_options = self.location.storage_options.clone();
        self.rt.block_on(open_vector_table(&uri, &storage_options))
    }

    /// Run one vector query against an already-opened cold table (search only).
    pub fn cold_search(&self, table: &Table, query: &[f32], k: usize, search: VectorSearch) -> Vec<VectorHit> {
        read_table(&self.rt, table, query, k, search)
    }
}

/// Cold-tier guard: one fresh connection + table open per instance, so
/// the shared `measure_cold` driver times the open and the first search
/// separately (mirror of the infino cold guards).
pub struct LanceVecColdGuard<'a> {
    index: &'a LanceVectorIndex,
    table: Table,
}

impl<'a> LanceVecColdGuard<'a> {
    pub fn open(index: &'a LanceVectorIndex) -> Self {
        Self {
            table: index.cold_open(),
            index,
        }
    }
}

impl VectorRead for LanceVecColdGuard<'_> {
    fn topk_global(
        &self,
        _column: &str,
        query: &[f32],
        k: usize,
        nprobe: usize,
        rerank: usize,
    ) -> Vec<(u32, f32)> {
        self.index
            .cold_search(
                &self.table,
                query,
                k,
                VectorSearch {
                    nprobe,
                    rerank_mult: rerank,
                },
            )
            .into_iter()
            .map(|h| (h.doc_id as u32, h.distance))
            .collect()
    }

    fn search_params(&self, nprobe: usize, rerank: usize) -> String {
        if nprobe == ENGINE_DEFAULT && rerank == ENGINE_DEFAULT {
            "engine defaults".into()
        } else {
            format!("p={nprobe}, r={rerank}")
        }
    }
}

fn delete_index(index: LanceVectorIndex) {
    if matches!(index.location.storage, LanceStorage::Remote) {
        let uri = index.location.uri.clone();
        let storage_options = index.location.storage_options.clone();
        index.rt.block_on(async move {
            let db = connect(&uri, &storage_options).await;
            let _ = db.drop_table(VEC_TABLE, &[]).await;
        });
    }
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
            ..Default::default()
        }
    }

    fn create(column: &str, dim: usize, metric: VectorMetric) -> Self::Index {
        create_index(column, dim, metric, LanceLocation::local())
    }

    fn write(index: &mut Self::Index, vectors: &[f32]) {
        write_index(index, vectors);
    }

    fn parallel_write(
        column: &str,
        vectors: &[f32],
        dim: usize,
        metric: VectorMetric,
        writers: usize,
    ) {
        parallel_write_index(column, vectors, dim, metric, writers, false);
    }

    fn read(index: &Self::Index, query: &[f32], k: usize, search: VectorSearch) -> Vec<VectorHit> {
        read_index(index, query, k, search)
    }

    fn close(index: &mut Self::Index) {
        index.table = None;
    }

    fn delete(index: Self::Index) {
        delete_index(index);
    }
}

impl VectorEngine for LanceS3VectorEngine {
    type Index = LanceVectorIndex;

    fn name() -> &'static str {
        lance_peer_label()
    }

    fn capabilities() -> Capabilities {
        LanceVectorEngine::capabilities()
    }

    fn create(column: &str, dim: usize, metric: VectorMetric) -> Self::Index {
        create_index(column, dim, metric, LanceLocation::object_store("vector"))
    }

    fn write(index: &mut Self::Index, vectors: &[f32]) {
        write_index(index, vectors);
    }

    fn parallel_write(
        column: &str,
        vectors: &[f32],
        dim: usize,
        metric: VectorMetric,
        writers: usize,
    ) {
        parallel_write_index(column, vectors, dim, metric, writers, true);
    }

    fn read(index: &Self::Index, query: &[f32], k: usize, search: VectorSearch) -> Vec<VectorHit> {
        read_index(index, query, k, search)
    }

    fn close(index: &mut Self::Index) {
        index.table = None;
    }

    fn delete(index: Self::Index) {
        delete_index(index);
    }
}
