// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Tantivy reference implementation of [`FtsEngine`].
//!
//! Tantivy is the in-memory full-text peer: it has no object-store
//! backend, so it appears only in the in-memory FTS comparison (never in
//! the S3 table). The adapter builds a RAM index over the same
//! `(doc_id, text)` corpus every other engine indexes and runs BM25
//! top-k through a [`BooleanQuery`] of [`TermQuery`]s — `Should` for OR,
//! `Must` for AND — so scoring and boolean semantics line up with the
//! other engines' batteries.

use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, TermQuery};
use tantivy::schema::{Field, IndexRecordOption, Schema, FAST, INDEXED, STORED, TEXT};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

use infino_bench_utils::harness::{BoolMode, Capabilities, FtsEngine, Hit};

/// Stable primary-key column carried alongside the indexed text so a hit
/// maps back to the corpus `doc_id` (engine-agnostic for recall grading).
const ID_FIELD: &str = "doc_id";

/// Indexing arena **per worker thread**. Tantivy enforces a ~15 MiB
/// floor per thread; 128 MiB keeps ingest from spilling segments too
/// often at 1M docs. The overall budget passed to Tantivy is this times
/// the worker count, so per-thread memory is constant whether we run 1
/// writer (the canonical `write`) or N (`parallel_write`).
const PER_THREAD_HEAP_BYTES: usize = 128 * 1024 * 1024;

/// Build the schema shared by `create`/`parallel_write`: a fast+stored
/// `doc_id` and the BM25-indexed text column.
fn build_schema(column: &str) -> (Schema, Field, Field) {
    let mut builder = Schema::builder();
    let id_field = builder.add_u64_field(ID_FIELD, FAST | STORED | INDEXED);
    let text_field = builder.add_text_field(column, TEXT);
    (builder.build(), id_field, text_field)
}

/// Build one in-RAM Tantivy index from `docs`, returning the finished
/// index and field handles. Shared by the queryable `write` and the
/// build-throughput probe.
fn build_index(column: &str, docs: &[(u64, &str)]) -> (Index, Field, Field) {
    let (schema, id_field, text_field) = build_schema(column);
    let index = Index::create_in_ram(schema);
    let mut writer: IndexWriter = index
        .writer_with_num_threads(1, PER_THREAD_HEAP_BYTES)
        .expect("tantivy index writer");
    for (id, text) in docs {
        let mut doc = TantivyDocument::default();
        doc.add_u64(id_field, *id);
        doc.add_text(text_field, *text);
        writer.add_document(doc).expect("tantivy add_document");
    }
    writer.commit().expect("tantivy commit");
    (index, id_field, text_field)
}

/// Tantivy as a comparison engine.
pub struct TantivyFtsEngine;

/// Sealed Tantivy FTS index: the RAM `Index`, its committed reader, and
/// the two field handles the query path needs.
pub struct TantivyFtsIndex {
    column: String,
    index: Option<Index>,
    id_field: Field,
    text_field: Field,
    reader: Option<IndexReader>,
}

impl TantivyFtsIndex {
    /// Index opened on the measured 1-writer artifact.
    pub fn index(&self) -> &Index {
        self.index.as_ref().expect("index requested before write")
    }

    /// Reader opened on the measured 1-writer artifact.
    pub fn reader(&self) -> &IndexReader {
        self.reader.as_ref().expect("reader requested before write")
    }
}

impl FtsEngine for TantivyFtsEngine {
    type Index = TantivyFtsIndex;

    fn name() -> &'static str {
        "tantivy"
    }

    fn capabilities() -> Capabilities {
        Capabilities {
            fts: true,
            vector: false,
            sql: false,
            hybrid: false,
        }
    }

    fn create(column: &str) -> Self::Index {
        let (_, id_field, text_field) = build_schema(column);
        TantivyFtsIndex {
            column: column.to_string(),
            index: None,
            id_field,
            text_field,
            reader: None,
        }
    }

    fn write(index: &mut Self::Index, docs: &[(u64, &str)]) {
        let (idx, id_field, text_field) = build_index(&index.column, docs);
        let reader = idx
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .expect("tantivy reader");
        index.index = Some(idx);
        index.id_field = id_field;
        index.text_field = text_field;
        index.reader = Some(reader);
    }

    fn parallel_write(column: &str, docs: &[(u64, &str)], writers: usize) {
        if writers <= 1 {
            std::hint::black_box(build_index(column, docs));
            return;
        }
        // Parallel build: shard the corpus across `writers` builders,
        // each emitting its own in-RAM index. Build-only — indices discarded.
        let shard_len = docs.len().div_ceil(writers);
        let shards: Vec<(Index, Field, Field)> = docs
            .chunks(shard_len)
            .map(|shard| build_index(column, shard))
            .collect();
        std::hint::black_box(shards);
    }

    fn read(index: &Self::Index, terms: &[&str], k: usize, mode: BoolMode) -> Vec<Hit> {
        let reader = index.reader();
        let searcher = reader.searcher();
        let occur = match mode {
            BoolMode::Or => Occur::Should,
            BoolMode::And => Occur::Must,
        };
        let subqueries: Vec<(Occur, Box<dyn Query>)> = terms
            .iter()
            .map(|t| {
                let term = Term::from_field_text(index.text_field, t);
                let q: Box<dyn Query> =
                    Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs));
                (occur, q)
            })
            .collect();
        let query = BooleanQuery::new(subqueries);

        let top = searcher
            .search(&query, &TopDocs::with_limit(k.max(1)).order_by_score())
            .expect("tantivy search");
        top.into_iter()
            .map(|(score, addr)| {
                let seg = searcher.segment_reader(addr.segment_ord);
                let ff = seg
                    .fast_fields()
                    .u64(ID_FIELD)
                    .expect("doc_id fast field");
                let doc_id = ff.first(addr.doc_id).expect("doc_id value");
                Hit { doc_id, score }
            })
            .collect()
    }

    fn close(index: &mut Self::Index) {
        index.reader = None;
    }

    fn delete(_index: Self::Index) {
        // Dropping the in-RAM index releases the artifact.
    }
}

