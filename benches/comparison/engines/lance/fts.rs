// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! LanceDB peer implementation of [`FtsEngine`].
//!
//! Builds a Lance dataset with an inverted (BM25/FTS) index over the
//! corpus text column. Stemming, stop-word removal, and ASCII folding
//! are disabled to match the simple tokenizer the other engines use.

use std::sync::Arc;

use arrow_array::{Float32Array, RecordBatch, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use datafusion::prelude::SessionContext;
use futures::TryStreamExt;
use lancedb::Table;
use lancedb::index::Index;
use lancedb::index::scalar::{
    BooleanQuery, FtsIndexBuilder, FtsQuery, FullTextSearchQuery, MatchQuery, Operator, PhraseQuery,
};
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use lancedb::table::datafusion::BaseTableAdapter;
use tokio::runtime::Runtime;

use super::location::{LanceLocation, LanceStorage, lance_peer_label};
use super::sql::scalar_i64;

use infino::superfile::fts::reader::BoolMode as InfinoBoolMode;
use infino::superfile::fts::tokenize::{AsciiLowerTokenizer, Tokenizer};
use infino_bench_utils::executors::fts::{FtsRead, to_infino_mode};
use infino_bench_utils::executors::payload_bytes;
use infino_bench_utils::harness::{BoolMode, Capabilities, FtsEngine, Hit};

const ID_COL: &str = "id";
const FTS_TABLE: &str = "fts";
const LANCE_TEXT_BATCH_ROWS: usize = 100_000;
/// Registration name + statement for the count phase's COUNT(*) over the
/// FTS-matched provider.
const FTS_COUNT_VIEW: &str = "fts_matches";
const FTS_COUNT_SQL: &str = "SELECT COUNT(*) FROM fts_matches";

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
    // Positions on: the shared battery includes phrase shapes, and lance
    // only executes `PhraseQuery` against a position-indexed column (the
    // position storage is the cost any lance user pays for phrase
    // support). Stemming/stop-words/folding stay off to match the simple
    // tokenizer the other engines use.
    let params = FtsIndexBuilder::default()
        .stem(false)
        .remove_stop_words(false)
        .ascii_folding(false)
        .with_position(true);
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

fn parallel_write_index(column: &str, docs: &[(u64, &str)], writers: usize, remote: bool) {
    if writers <= 1 {
        let rt = new_runtime();
        let location = if remote {
            LanceLocation::object_store("fts")
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
                    let location = if remote {
                        LanceLocation::object_store(&format!("fts-shard-{i}"))
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

/// Build the lance query DSL for one battery shape. Clause polarity
/// (sigils, phrases, And-mode folding of bare terms into musts) is
/// decided by infino's OWN tokenizer — one parser rules both engines —
/// and only the DSL construction below is lance-specific.
fn battery_query(column: &str, query: &str, mode: InfinoBoolMode) -> FtsQuery {
    let clauses = AsciiLowerTokenizer.parse(query).into_clauses(mode);
    let col = || Some(column.to_string());
    let join = |terms: &[std::borrow::Cow<'_, str>]| {
        terms
            .iter()
            .map(|t| t.as_ref())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let phrase = |tokens: &Vec<std::borrow::Cow<'_, str>>| {
        FtsQuery::Phrase(PhraseQuery::new(join(tokens)).with_column(col()))
    };

    let simple = clauses.must_phrases.is_empty()
        && clauses.should_phrases.is_empty()
        && clauses.negatives.is_empty()
        && clauses.negative_phrases.is_empty();
    if simple && clauses.musts.is_empty() {
        return FtsQuery::Match(
            MatchQuery::new(join(&clauses.shoulds))
                .with_column(col())
                .with_operator(Operator::Or),
        );
    }
    if simple && clauses.shoulds.is_empty() {
        return FtsQuery::Match(
            MatchQuery::new(join(&clauses.musts))
                .with_column(col())
                .with_operator(Operator::And),
        );
    }

    let mut bq = BooleanQuery::new(std::iter::empty());
    if !clauses.musts.is_empty() {
        bq = bq.with_must(FtsQuery::Match(
            MatchQuery::new(join(&clauses.musts))
                .with_column(col())
                .with_operator(Operator::And),
        ));
    }
    for p in &clauses.must_phrases {
        bq = bq.with_must(phrase(p));
    }
    if !clauses.shoulds.is_empty() {
        bq = bq.with_should(FtsQuery::Match(
            MatchQuery::new(join(&clauses.shoulds))
                .with_column(col())
                .with_operator(Operator::Or),
        ));
    }
    for p in &clauses.should_phrases {
        bq = bq.with_should(phrase(p));
    }
    if !clauses.negatives.is_empty() {
        bq = bq.with_must_not(FtsQuery::Match(
            MatchQuery::new(join(&clauses.negatives))
                .with_column(col())
                .with_operator(Operator::Or),
        ));
    }
    for p in &clauses.negative_phrases {
        bq = bq.with_must_not(phrase(p));
    }
    // A lone must clause needs no boolean wrapper.
    if bq.should.is_empty() && bq.must_not.is_empty() && bq.must.len() == 1 {
        return bq.must.pop().expect("single must clause");
    }
    FtsQuery::Boolean(bq)
}

/// Run one battery query and return the raw result batches. `fetched`
/// selects the fetch phase (id + the searched text column) vs the
/// search phase (id only; `_score` is system-added on both).
fn fts_query_batches(
    rt: &Runtime,
    table: &Table,
    column: &str,
    query: &str,
    k: usize,
    mode: InfinoBoolMode,
    fetched: bool,
) -> Vec<RecordBatch> {
    let fts_query = FullTextSearchQuery::new_query(battery_query(column, query, mode));
    let select = if fetched {
        vec![ID_COL.to_string(), column.to_string()]
    } else {
        vec![ID_COL.to_string()]
    };
    rt.block_on(async {
        let stream = table
            .query()
            .full_text_search(fts_query)
            .select(Select::Columns(select))
            .limit(k.max(1))
            .execute()
            .await
            .expect("fts query execute");
        stream.try_collect().await.expect("collect stream")
    })
}

fn read_table(
    rt: &Runtime,
    table: &Table,
    column: &str,
    terms: &[&str],
    k: usize,
    mode: BoolMode,
) -> Vec<Hit> {
    let batches = fts_query_batches(
        rt,
        table,
        column,
        &terms.join(" "),
        k,
        to_infino_mode(mode),
        false,
    );
    {
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
    }
}

fn read_index(index: &LanceFtsIndex, terms: &[&str], k: usize, mode: BoolMode) -> Vec<Hit> {
    read_table(&index.rt, index.table(), &index.column, terms, k, mode)
}

impl LanceFtsIndex {
    /// Reopen the same object-store artifact and run one FTS query. Used by the
    /// comparison cold tier so cold does not include rebuild time.
    pub fn cold_read(&self, terms: &[&str], k: usize, mode: BoolMode) -> Vec<Hit> {
        let uri = self.location.uri.clone();
        let storage_options = self.location.storage_options.clone();
        let table = self.rt.block_on(open_fts_table(&uri, &storage_options));
        read_table(&self.rt, &table, &self.column, terms, k, mode)
    }

    /// Open the cold object-store artifact (connect + open_table) without querying.
    /// The cold tier times only the search, so the open is excluded —
    /// matching the Infino cold path, which opens its consumer outside the
    /// timed region.
    pub fn cold_open(&self) -> Table {
        let uri = self.location.uri.clone();
        let storage_options = self.location.storage_options.clone();
        self.rt.block_on(open_fts_table(&uri, &storage_options))
    }

    /// Run one FTS query against an already-opened cold table (search only).
    pub fn cold_search(&self, table: &Table, terms: &[&str], k: usize, mode: BoolMode) -> Vec<Hit> {
        read_table(&self.rt, table, &self.column, terms, k, mode)
    }
}

/// Both `FtsRead` phases for a given open lance table — the shared
/// protocol batteries (`exec_fts::measure_warm` / `measure_cold` /
/// `measure_count`) drive the peer through exactly these calls.
fn fts_read_rows(
    rt: &Runtime,
    table: &Table,
    column: &str,
    query: &str,
    k: usize,
    mode: InfinoBoolMode,
    fetched: bool,
) -> usize {
    fts_query_batches(rt, table, column, query, k, mode, fetched)
        .iter()
        .map(|b| b.num_rows())
        .sum()
}

fn fts_read_payloads(
    rt: &Runtime,
    table: &Table,
    column: &str,
    query: &str,
    k: usize,
    mode: InfinoBoolMode,
) -> ((u64, u64), (u64, u64)) {
    let search = fts_query_batches(rt, table, column, query, k, mode, false);
    let fetched = fts_query_batches(rt, table, column, query, k, mode, true);
    (payload_bytes(&search), payload_bytes(&fetched))
}

fn fts_read_count(
    rt: &Runtime,
    table: &Table,
    column: &str,
    query: &str,
    mode: InfinoBoolMode,
) -> u64 {
    // Normal-SQL count of the FTS match: attach the query to the live
    // lance provider and let DataFusion aggregate COUNT(*) in the engine
    // pipeline — only the scalar crosses, same as infino's count path
    // returns a count, not ids.
    let fts_query = FullTextSearchQuery::new_query(battery_query(column, query, mode));
    rt.block_on(async {
        let adapter = BaseTableAdapter::try_new(table.base_table().clone())
            .await
            .expect("adapt lance table for datafusion")
            .with_fts_query(fts_query);
        let ctx = SessionContext::new();
        ctx.register_table(FTS_COUNT_VIEW, Arc::new(adapter))
            .expect("register fts count provider");
        let batches = ctx
            .sql(FTS_COUNT_SQL)
            .await
            .expect("plan fts count")
            .collect()
            .await
            .expect("collect fts count");
        scalar_i64(&batches) as u64
    })
}

impl FtsRead for LanceFtsIndex {
    fn bm25_rows(&self, column: &str, query: &str, k: usize, mode: InfinoBoolMode) -> usize {
        fts_read_rows(&self.rt, self.table(), column, query, k, mode, false)
    }

    fn bm25_rows_fetched(
        &self,
        column: &str,
        query: &str,
        k: usize,
        mode: InfinoBoolMode,
    ) -> usize {
        fts_read_rows(&self.rt, self.table(), column, query, k, mode, true)
    }

    fn bm25_payloads(
        &self,
        column: &str,
        query: &str,
        k: usize,
        mode: InfinoBoolMode,
    ) -> ((u64, u64), (u64, u64)) {
        fts_read_payloads(&self.rt, self.table(), column, query, k, mode)
    }

    fn count_matching(&self, column: &str, query: &str, mode: InfinoBoolMode) -> u64 {
        fts_read_count(&self.rt, self.table(), column, query, mode)
    }
}

/// Cold-tier guard: one fresh connection + table open per instance, so
/// the shared `exec_fts::measure_cold` driver times the open and the
/// first query separately, for both phases on separate fresh opens.
pub struct LanceFtsColdGuard<'a> {
    index: &'a LanceFtsIndex,
    table: Table,
}

impl<'a> LanceFtsColdGuard<'a> {
    pub fn open(index: &'a LanceFtsIndex) -> Self {
        Self {
            table: index.cold_open(),
            index,
        }
    }
}

impl FtsRead for LanceFtsColdGuard<'_> {
    fn bm25_rows(&self, column: &str, query: &str, k: usize, mode: InfinoBoolMode) -> usize {
        fts_read_rows(&self.index.rt, &self.table, column, query, k, mode, false)
    }

    fn bm25_rows_fetched(
        &self,
        column: &str,
        query: &str,
        k: usize,
        mode: InfinoBoolMode,
    ) -> usize {
        fts_read_rows(&self.index.rt, &self.table, column, query, k, mode, true)
    }

    fn bm25_payloads(
        &self,
        column: &str,
        query: &str,
        k: usize,
        mode: InfinoBoolMode,
    ) -> ((u64, u64), (u64, u64)) {
        fts_read_payloads(&self.index.rt, &self.table, column, query, k, mode)
    }

    fn count_matching(&self, column: &str, query: &str, mode: InfinoBoolMode) -> u64 {
        fts_read_count(&self.index.rt, &self.table, column, query, mode)
    }
}

fn delete_index(index: LanceFtsIndex) {
    if matches!(index.location.storage, LanceStorage::Remote) {
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
            ..Default::default()
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
        lance_peer_label()
    }

    fn capabilities() -> Capabilities {
        LanceFtsEngine::capabilities()
    }

    fn create(column: &str) -> Self::Index {
        create_index(column, LanceLocation::object_store("fts"))
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
