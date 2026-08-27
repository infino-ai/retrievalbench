// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Table-level append and delete: Infino `append`/`delete` (each a
//! commit) vs Lance `Table::add` / `Table::delete`, timed with
//! [`PeakSampler`].
//!
//! Superfile insert/remove is not this cell. Infino superfiles are
//! sealed; new rows land as a new superfile on the manifest. Lance
//! appends into the live dataset (IVF_PQ is not rebuilt — new rows stay
//! searchable through Lance's unindexed fallback).

use std::sync::Arc;
use std::time::Duration;

use arrow_array::{FixedSizeListArray, Float32Array, LargeStringArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use datafusion::prelude::{col, lit};
use infino::{IndexSpec, Metric, connect};
use infino_bench_utils::corpus;
use infino_bench_utils::cpu;
use infino_bench_utils::harness::{VectorEngine, VectorMetric};
use infino_bench_utils::ingest::supertable::{self, VEC_COLUMN};
use infino_bench_utils::markdown::{fmt_count, fmt_time};
use infino_bench_utils::report::{Better, Block, Report, Section, metric, text};
use infino_bench_utils::rss::{self, PeakSampler, RssStats};
use retrievalbench::LanceVectorEngine;
use retrievalbench::lance::vector::LanceVectorIndex;

/// Single-row append / delete, matching the in-memory insert n=1 cell.
const APPEND_SINGLE: usize = 1;
/// Batch append, matching the in-memory insert n=100 cell.
const APPEND_BATCH: usize = 100;
/// Single-row mutations averaged over this many committed ops so the
/// process-CPU sampler sees a measurable window.
const N_MUTATIONS: usize = 8;
const EXTRA_QUERY_SEED: u64 = 1;
const EXTRA_QUERY_SIGMA: f32 = 0.01;
/// Marker column used for Infino delete predicates. `_id` is injected
/// by the table and is not in the declared schema.
const MARKER_COL: &str = "row_key";
const NS_PER_SEC: f64 = 1e9;

struct OpSample {
    wall: Duration,
    rss: RssStats,
}

fn measure(f: impl FnOnce()) -> OpSample {
    let sampler = PeakSampler::start_default();
    let ((), wall, _) = cpu::timed(f);
    OpSample {
        wall,
        rss: sampler.stop_stats(),
    }
}

fn vector_field(dim: usize) -> DataType {
    DataType::FixedSizeList(
        Arc::new(Field::new("item", DataType::Float32, true)),
        dim as i32,
    )
}

fn infino_schema(dim: usize) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(MARKER_COL, DataType::LargeUtf8, false),
        Field::new(VEC_COLUMN, vector_field(dim), false),
    ]))
}

fn infino_batch(schema: Arc<Schema>, dim: usize, keys: &[String], vectors: &[f32]) -> RecordBatch {
    let n = keys.len();
    assert_eq!(vectors.len(), n * dim);
    let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    let flat = Float32Array::from(vectors.to_vec());
    let fsl = FixedSizeListArray::try_new(
        Arc::new(Field::new("item", DataType::Float32, true)),
        dim as i32,
        Arc::new(flat),
        None,
    )
    .expect("infino FixedSizeListArray");
    RecordBatch::try_new(
        schema,
        vec![Arc::new(LargeStringArray::from(key_refs)), Arc::new(fsl)],
    )
    .expect("infino RecordBatch")
}

fn marker(prefix: &str, i: usize) -> String {
    format!("{prefix}-{i:08}")
}

fn sample_row(label: &str, ops: usize, sample: &OpSample) -> Vec<infino_bench_utils::report::Cell> {
    let wall_ns = sample.wall.as_secs_f64() / ops as f64 * NS_PER_SEC;
    vec![
        text(label),
        metric(wall_ns, fmt_time(wall_ns), Better::Lower),
        metric(
            sample.rss.peak_rss_bytes as f64,
            rss::fmt_bytes(sample.rss.peak_rss_bytes),
            Better::Lower,
        ),
    ]
}

fn infino_cells(
    seed: &[f32],
    extra: &[f32],
    dim: usize,
) -> Vec<Vec<infino_bench_utils::report::Cell>> {
    let n_seed = seed.len() / dim;
    let schema = infino_schema(dim);
    let dir = tempfile::TempDir::new().expect("infino table-write tempdir");
    let uri = dir.path().to_str().expect("utf8 temp path");
    let db = connect(uri).expect("infino connect");
    let table = db
        .create_table(
            "docs",
            schema.clone(),
            IndexSpec::new().vector(VEC_COLUMN, dim, Metric::Cosine),
        )
        .expect("create_table");

    let seed_keys: Vec<String> = (0..n_seed).map(|i| marker("seed", i)).collect();
    table
        .append(&infino_batch(schema.clone(), dim, &seed_keys, seed))
        .expect("seed append");

    let extra_n = extra.len() / dim;
    assert!(
        extra_n >= APPEND_SINGLE * N_MUTATIONS + APPEND_BATCH,
        "need leftover rows for measured appends"
    );

    let mut extra_off = 0;
    let append_1 = measure(|| {
        for i in 0..N_MUTATIONS {
            let key = marker("append1", i);
            let keys = [key];
            let start = (extra_off + i) * dim;
            let batch = infino_batch(schema.clone(), dim, &keys, &extra[start..start + dim]);
            table.append(&batch).expect("append 1 row");
        }
    });
    extra_off += N_MUTATIONS * APPEND_SINGLE;

    let batch_keys: Vec<String> = (0..APPEND_BATCH).map(|i| marker("append100", i)).collect();
    let batch_flat = extra[extra_off * dim..(extra_off + APPEND_BATCH) * dim].to_vec();
    let append_100 = measure(|| {
        table
            .append(&infino_batch(schema.clone(), dim, &batch_keys, &batch_flat))
            .expect("append 100 rows");
    });

    let delete_1 = measure(|| {
        for i in 0..N_MUTATIONS {
            table
                .delete(col(MARKER_COL).eq(lit(marker("seed", i))))
                .expect("delete 1 row");
        }
    });
    let delete_batch: Vec<_> = (N_MUTATIONS..N_MUTATIONS + APPEND_BATCH)
        .map(|i| lit(marker("seed", i)))
        .collect();
    let delete_100 = measure(|| {
        table
            .delete(col(MARKER_COL).in_list(delete_batch, false))
            .expect("delete 100 rows");
    });

    vec![
        sample_row("infino append 1 row", N_MUTATIONS, &append_1),
        sample_row("infino append 100 rows", 1, &append_100),
        sample_row("infino delete 1 row", N_MUTATIONS, &delete_1),
        sample_row("infino delete 100 rows", 1, &delete_100),
    ]
}

fn lance_cells(
    seed: &[f32],
    extra: &[f32],
    dim: usize,
) -> Vec<Vec<infino_bench_utils::report::Cell>> {
    let n_seed = seed.len() / dim;
    let mut index: LanceVectorIndex =
        LanceVectorEngine::create(VEC_COLUMN, dim, VectorMetric::Cosine);
    LanceVectorEngine::write(&mut index, seed);

    let extra_n = extra.len() / dim;
    assert!(
        extra_n >= APPEND_SINGLE * N_MUTATIONS + APPEND_BATCH,
        "need leftover rows for measured appends"
    );

    let mut next_id = n_seed as u64;
    let append_1 = measure(|| {
        for i in 0..N_MUTATIONS {
            let start = i * dim;
            index.add_vectors(&extra[start..start + dim], next_id);
            next_id += 1;
        }
    });
    let batch_off = N_MUTATIONS * APPEND_SINGLE;
    let append_100 = measure(|| {
        index.add_vectors(
            &extra[batch_off * dim..(batch_off + APPEND_BATCH) * dim],
            next_id,
        );
    });
    let delete_1 = measure(|| {
        let ids: Vec<u64> = (0..N_MUTATIONS as u64).collect();
        for id in ids {
            index.delete_ids(&[id]);
        }
    });
    let delete_100 = measure(|| {
        let ids: Vec<u64> = (N_MUTATIONS as u64..(N_MUTATIONS + APPEND_BATCH) as u64).collect();
        index.delete_ids(&ids);
    });

    LanceVectorEngine::close(&mut index);
    LanceVectorEngine::delete(index);

    vec![
        sample_row("lancedb add 1 row", N_MUTATIONS, &append_1),
        sample_row("lancedb add 100 rows", 1, &append_100),
        sample_row("lancedb delete 1 row", N_MUTATIONS, &delete_1),
        sample_row("lancedb delete 100 rows", 1, &delete_100),
    ]
}

pub fn run() {
    let prepared = supertable::prepare_corpus(supertable::Modality::Vector);
    let vectors = prepared
        .vectors()
        .expect("vector modality prepares a vector corpus");
    let dim = corpus::dim();
    let available_docs = vectors.n_docs();
    let extra_needed = APPEND_SINGLE * N_MUTATIONS + APPEND_BATCH;
    let seed_n = supertable::n_docs();
    assert!(
        available_docs >= seed_n,
        "corpus too small for table-write seed (need {seed_n}, have {available_docs})"
    );
    let seed = &vectors.as_slice()[..seed_n * dim];
    let generated_extra;
    let extra = if available_docs >= seed_n + extra_needed {
        &vectors.as_slice()[seed_n * dim..(seed_n + extra_needed) * dim]
    } else {
        generated_extra = corpus::generate_realistic_queries(
            seed,
            seed_n,
            extra_needed,
            EXTRA_QUERY_SEED,
            true,
            EXTRA_QUERY_SIGMA,
        )
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        generated_extra.as_slice()
    };

    eprintln!(
        "[comparison-table-writes] seed {} docs × dim={}, then append 1 / 100 and delete 1 / 100 \
         (Infino append+commit vs Lance Table::add / Table::delete)...",
        fmt_count(seed_n),
        dim
    );

    let mut rows = infino_cells(seed, extra, dim);
    rows.extend(lance_cells(seed, extra, dim));

    let mut report = Report::load("comparison-supertable-vector-writes");
    report.emit(&Section {
        anchor: "comparison/supertable/vector/writes".into(),
        title: format!(
            "Supertable writes — Infino append/delete vs Lance add/delete ({} seed docs × dim={})",
            fmt_count(seed_n),
            dim
        ),
        note: "Infino `append` / `delete` each commit (new superfile or tombstone). \
               Lance `Table::add` / `Table::delete` mutate the live dataset; IVF_PQ is \
               not rebuilt on add. Single-row ops are averaged over a short loop. Peak \
               RSS is process-wide (PeakSampler) over that window. This is the table \
               path — not a sealed-superfile rebuild."
            .into(),
        blocks: vec![Block {
            subtitle: "Append / delete".into(),
            headers: vec!["Op".into(), "Wall/op".into(), "Peak RSS".into()],
            rows,
        }],
    });
    report.save();
}
