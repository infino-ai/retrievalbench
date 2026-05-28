//! Lance head-to-head helpers: builds a Lance `Table` from the same
//! `Vec<f32>` corpus we hand to infino, runs parameterized kNN
//! searches, and calibrates Lance's `(probe, refine)` knob to the
//! lowest p50 latency that hits a given recall@10 target.
//!
//! Lance is async; bench loops are sync, so callers pass a
//! `tokio::runtime::Runtime` and we `block_on` each call. One runtime
//! shared across the whole bench keeps the cost off the hot path.
//!
//! Lance was compiled against arrow 57 while the infino tree pulls
//! arrow 58. The bench reaches arrow 57 through the renamed
//! `arrow_array_lance` / `arrow_schema_lance` deps so we can hand
//! Lance the batch shape it expects.
//!
//! ## Why no infino measurement helpers here
//!
//! Infino-only timing, calibration, and corpus generators all live in
//! `infino::test_helpers::bench_corpus` (re-exported via this crate's
//! `corpus` module). Calling `crate::corpus::calibrate_infino(...)`,
//! `crate::corpus::recall_at_k(...)`, etc. gives both repos a single
//! source of truth so retrievalbench's head-to-head tables read
//! infino's *published* numbers (from `../infino/target/criterion/...`)
//! rather than re-measuring infino in this process.

#![allow(clippy::too_many_arguments)]

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use arrow_array_lance::{
    Array, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, RecordBatchReader,
    StringArray, UInt32Array,
};
use arrow_schema_lance::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::DistanceType;
use lancedb::Table;
use lancedb::index::Index;
use lancedb::index::scalar::{FtsIndexBuilder, FtsQuery, FullTextSearchQuery, MatchQuery, Operator};
use lancedb::index::vector::IvfPqIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase};
use tokio::runtime::Runtime;

use crate::corpus::{Calibrated, DIM, Hit, p50_micros, recall_at_k};

/// Build the Lance table at `path` with an IVF-PQ index, return it
/// open and ready for queries. Times the whole pipeline (data load
/// + IVF train + index write).
pub fn build_lance_table(
    rt: &Runtime,
    path: &Path,
    vectors: &[f32],
    n_docs: usize,
    n_partitions: u32,
    n_sub_vectors: u32,
) -> (Table, std::time::Duration) {
    let t0 = Instant::now();
    let table = rt.block_on(async move {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::UInt32, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    DIM as i32,
                ),
                false,
            ),
        ]));

        let ids = UInt32Array::from((0..n_docs as u32).collect::<Vec<_>>());
        let flat = Float32Array::from(vectors.to_vec());
        let item_field = Arc::new(Field::new("item", DataType::Float32, true));
        let fsl = FixedSizeListArray::try_new(item_field, DIM as i32, Arc::new(flat), None)
            .expect("build FixedSizeListArray");
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(fsl)])
            .expect("build RecordBatch");
        let reader: Box<dyn RecordBatchReader + Send> =
            Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));

        let db = lancedb::connect(path.to_str().expect("path to_str"))
            .execute()
            .await
            .expect("await async result");
        let table = db
            .create_table("v", reader)
            .execute()
            .await
            .expect("create lance table");

        table
            .create_index(
                &["vector"],
                Index::IvfPq(
                    IvfPqIndexBuilder::default()
                        .num_partitions(n_partitions)
                        .num_sub_vectors(n_sub_vectors)
                        .distance_type(DistanceType::Cosine),
                ),
            )
            .execute()
            .await
            .expect("await async result");
        table
    });
    (table, t0.elapsed())
}

/// One Lance kNN call. Returns `(id, distance)` pairs sorted by
/// distance ascending — same shape as `infino::VectorReader::search`.
pub fn search_lance(
    rt: &Runtime,
    table: &Table,
    query: &[f32],
    k: usize,
    nprobes: usize,
    refine_factor: u32,
) -> Vec<Hit> {
    rt.block_on(async move {
        let q = query.to_vec();
        let stream = table
            .query()
            .nearest_to(q)
            .expect("nearest_to")
            .nprobes(nprobes)
            .refine_factor(refine_factor)
            .limit(k)
            .execute()
            .await
            .expect("await async result");
        let batches: Vec<RecordBatch> = stream.try_collect().await.expect("collect stream");
        let mut out = Vec::with_capacity(k);
        for b in batches {
            let id_col = b
                .column_by_name("id")
                .expect("column by name")
                .as_any()
                .downcast_ref::<UInt32Array>()
                .expect("downcast");
            let dist_col = b
                .column_by_name("_distance")
                .expect("column by name")
                .as_any()
                .downcast_ref::<Float32Array>()
                .expect("downcast");
            for i in 0..b.num_rows() {
                out.push((id_col.value(i), dist_col.value(i)));
            }
        }
        out
    })
}

pub fn mean_recall_lance(
    rt: &Runtime,
    table: &Table,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
    k: usize,
    nprobes: usize,
    refine_factor: u32,
) -> f32 {
    let mut sum = 0f32;
    for (q, t) in queries.iter().zip(truths) {
        let hits = search_lance(rt, table, q, k, nprobes, refine_factor);
        sum += recall_at_k(&hits, t);
    }
    sum / queries.len() as f32
}

/// Sweep a `(probe, refine)` grid for Lance, return the lowest-p50
/// point that hits `recall ≥ target_recall`. `None` if no grid point
/// meets the target.
pub fn calibrate_lance(
    rt: &Runtime,
    table: &Table,
    queries: &[Vec<f32>],
    truths: &[Vec<u32>],
    target_recall: f32,
    probes: &[usize],
    refines: &[u32],
    p50_iter: usize,
    k: usize,
) -> Option<Calibrated> {
    let mut best: Option<Calibrated> = None;
    let mut peak_recall = 0f32;
    for &probe in probes {
        for &refine in refines {
            let recall = mean_recall_lance(rt, table, queries, truths, k, probe, refine);
            if recall > peak_recall {
                peak_recall = recall;
            }
            if recall < target_recall {
                continue;
            }
            let q = &queries[0];
            let p50 = p50_micros(
                || {
                    let _ = search_lance(rt, table, q, k, probe, refine);
                },
                p50_iter,
            );
            let cand = Calibrated {
                probe,
                refine: refine as usize,
                recall,
                p50_micros: p50,
            };
            best = match best {
                None => Some(cand),
                Some(b) if cand.p50_micros < b.p50_micros => Some(cand),
                Some(b) => Some(b),
            };
        }
    }
    if best.is_none() {
        eprintln!(
            "    [lance] no point hit recall ≥ {target_recall:.2}; peak observed = {peak_recall:.3}"
        );
    }
    best
}

/// Default `num_sub_vectors` for IVF-PQ at our DIM. Matches Lance's
/// own recommended setting for dim=384: 64 sub-vectors of 6 dims each,
/// 8-bit codes (256 centroids per subvec). Finer-grained quantization
/// than dim/8 = 48 → marginally better recall per probe at the cost of
/// ~33% larger PQ codes (64 B vs 48 B / vec).
pub fn default_n_sub_vectors() -> u32 {
    64
}

/// Build a Lance table at `path` with a full-text search (FTS / inverted)
/// index, return it open and ready for queries. Times the whole pipeline
/// (data load + index write).
pub fn build_lance_fts_table(
    rt: &Runtime,
    path: &Path,
    docs: &[String],
    n_docs: usize,
) -> (Table, std::time::Duration) {
    let t0 = Instant::now();
    let table = rt.block_on(async move {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::UInt32, false),
            Field::new("text", DataType::Utf8, false),
        ]));

        let ids = UInt32Array::from((0..n_docs as u32).collect::<Vec<_>>());
        let texts = StringArray::from(docs.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(texts)])
            .expect("build RecordBatch");
        let reader: Box<dyn RecordBatchReader + Send> =
            Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));

        let db = lancedb::connect(path.to_str().expect("path to_str"))
            .execute()
            .await
            .expect("await async result");
        let table = db
            .create_table("fts", reader)
            .execute()
            .await
            .expect("create lance table");

        // Disable stemming, stop-word removal, and ASCII folding: our
        // synthetic corpus terms (e.g. "term00001") get no recall benefit
        // from these filters, and each filter adds query-time tokenization
        // overhead. This matches what the Tantivy bench uses: only
        // SimpleTokenizer + LowerCaser.
        let fts_params = FtsIndexBuilder::default()
            .stem(false)
            .remove_stop_words(false)
            .ascii_folding(false);
        table
            .create_index(&["text"], Index::FTS(fts_params))
            .execute()
            .await
            .expect("await async result");

        // Prewarm: load all FTS posting lists into memory so search
        // measurements don't pay cold I/O latency. The index name is
        // the column name + "_idx" (lancedb convention).
        let idx_name = table
            .list_indices()
            .await
            .expect("await async result")
            .into_iter()
            .next()
            .map(|c| c.name)
            .unwrap_or_else(|| "text_idx".into());
        table
            .prewarm_index(&idx_name)
            .await
            .expect("await async result");

        table
    });
    (table, t0.elapsed())
}

/// Execute a pre-built `FullTextSearchQuery`. Callers construct the query
/// once outside `b.iter()` and `.clone()` it per iteration so that query
/// object allocation is not part of the measured time — same pattern as
/// Tantivy where `parser.parse_query(...)` is called before the bench
/// group and a `&dyn Query` reference is reused across iterations.
/// `FullTextSearchQuery` and all `FtsQuery` variants derive `Clone`.
pub fn search_lance_fts_query(
    rt: &Runtime,
    table: &Table,
    fts_query: FullTextSearchQuery,
    k: usize,
) -> Vec<(u32, f32)> {
    rt.block_on(async move {
        let stream = table
            .query()
            .full_text_search(fts_query)
            .limit(k)
            .execute()
            .await
            .expect("await async result");
        let batches: Vec<RecordBatch> = stream.try_collect().await.expect("collect stream");
        let mut out = Vec::with_capacity(k);
        for b in batches {
            let id_col = b
                .column_by_name("id")
                .expect("column by name")
                .as_any()
                .downcast_ref::<UInt32Array>()
                .expect("downcast");
            let score_col = b
                .column_by_name("_score")
                .expect("column by name")
                .as_any()
                .downcast_ref::<Float32Array>()
                .expect("downcast");
            for i in 0..b.num_rows() {
                out.push((id_col.value(i), score_col.value(i)));
            }
        }
        out
    })
}

/// Build an OR `FullTextSearchQuery` for the given query string with WAND
/// enabled. Construct this once outside `b.iter()` and pass `.clone()` to
/// `search_lance_fts_query` each iteration.
pub fn make_lance_fts_or_query(query: &str) -> FullTextSearchQuery {
    FullTextSearchQuery::new(query.to_string()).wand_factor(Some(1.0))
}

/// Build an AND `FullTextSearchQuery` for the given space-separated terms.
/// Construct this once outside `b.iter()` and pass `.clone()` to
/// `search_lance_fts_query` each iteration.
pub fn make_lance_fts_and_query(terms: &[String]) -> FullTextSearchQuery {
    let joined = terms.join(" ");
    let match_q = MatchQuery::new(joined).with_operator(Operator::And);
    FullTextSearchQuery::new_query(FtsQuery::Match(match_q)).wand_factor(Some(1.0))
}

/// One Lance FTS OR call. Returns `(id, score)` pairs. Uses WAND
/// (`wand_factor = 1.0`) for best query performance. The query string
/// is treated as an OR of its tokens by the underlying inverted index.
pub fn search_lance_fts(
    rt: &Runtime,
    table: &Table,
    query: &str,
    k: usize,
) -> Vec<(u32, f32)> {
    rt.block_on(async move {
        let fts_query = FullTextSearchQuery::new(query.to_string()).wand_factor(Some(1.0));
        let stream = table
            .query()
            .full_text_search(fts_query)
            .limit(k)
            .execute()
            .await
            .expect("await async result");
        let batches: Vec<RecordBatch> = stream.try_collect().await.expect("collect stream");
        let mut out = Vec::with_capacity(k);
        for b in batches {
            let id_col = b
                .column_by_name("id")
                .expect("column by name")
                .as_any()
                .downcast_ref::<UInt32Array>()
                .expect("downcast");
            let score_col = b
                .column_by_name("_score")
                .expect("column by name")
                .as_any()
                .downcast_ref::<Float32Array>()
                .expect("downcast");
            for i in 0..b.num_rows() {
                out.push((id_col.value(i), score_col.value(i)));
            }
        }
        out
    })
}
