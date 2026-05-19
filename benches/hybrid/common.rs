//! Shared fixtures for the hybrid (FTS + vector) supertable benches.
//!
//! A "hybrid" workload here means a [`Supertable`] with **both** an
//! FTS-indexed column and a vector-indexed column on the same docs —
//! the realistic production shape. Per-topic supertable benches under
//! `fts/supertable/` and `vector/supertable/` test single-topic
//! supertables for tighter cross-engine isolation; this bundle owns
//! the full hybrid shape: build throughput across writer-pool sizes,
//! append latency, query latency across both query types, and the
//! dual-pool reader-p99-under-writer-load epilogue.

use std::sync::{Arc, OnceLock};

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, LargeStringArray, RecordBatch,
};
use arrow_schema::{DataType, Field, Schema};
use infino::superfile::builder::{FtsConfig, VectorConfig};
use infino::superfile::fts::tokenize::Tokenizer;
use infino::superfile::vector::distance::Metric;
use infino::test_helpers::default_tokenizer;
use infino::supertable::{Supertable, SupertableOptions};

/// Number of buffer chunks to use during the build bench. Chosen
/// to be >= `num_cpus()` on typical laptops, so each writer-pool
/// configuration in {1, 4, num_cpus} actually exercises its full
/// shard width — `commit()` rayon-shards across the buffer's
/// chunked batches, so 1-batch buffers can't exercise > 1 thread.
pub const N_BUFFER_CHUNKS: usize = 16;

/// k for top-k queries.
pub const TOP_K: usize = 10;

/// Cached corpus: titles + flat vector buffer + n_cent.
pub struct Corpus {
    pub titles: Vec<String>,
    pub vectors: Vec<f32>,
    pub n_cent: usize,
    pub n: usize,
}

static CORPUS: OnceLock<Corpus> = OnceLock::new();

pub fn corpus() -> &'static Corpus {
    CORPUS.get_or_init(|| {
        let n = crate::corpus::n_docs();
        let n_cent = crate::corpus::n_cent(n);
        Corpus {
            titles: crate::corpus::generate_text_corpus(n, 1),
            vectors: crate::corpus::generate_vector_corpus(n, n_cent, 1, true),
            n_cent,
            n,
        }
    })
}

/// Pre-built per-segment `RecordBatch`es, one per buffer chunk.
/// Constructing these is expensive (each chunk allocates a
/// `Float32Array` from a slice of the ~`DIM·n/N_BUFFER_CHUNKS`
/// floats) — pre-building once keeps the bench measuring
/// commit-time work, not Arrow array construction.
static BATCHES: OnceLock<Vec<RecordBatch>> = OnceLock::new();

pub fn batches() -> &'static [RecordBatch] {
    BATCHES.get_or_init(|| {
        let c = corpus();
        (0..N_BUFFER_CHUNKS)
            .map(|i| batch_chunk(c, i, N_BUFFER_CHUNKS))
            .filter(|b| b.num_rows() > 0)
            .collect()
    })
}

pub fn supertable_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("title", DataType::LargeUtf8, false),
        Field::new(
            "emb",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                crate::corpus::DIM as i32,
            ),
            false,
        ),
    ]))
}

pub fn supertable_options(n_cent: usize, writer_threads: usize) -> SupertableOptions {
    let writer_pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(writer_threads)
            .thread_name(|i| format!("hybrid-bench-writer-{i}"))
            .build()
            .expect("writer pool"),
    );
    let tk: Arc<dyn Tokenizer> = default_tokenizer();
    SupertableOptions::new(
        supertable_schema(),
        vec![FtsConfig {
            column: "title".into(),
        }],
        vec![VectorConfig {
            column: "emb".into(),
            dim: crate::corpus::DIM,
            n_cent,
            rot_seed: 7,
            metric: Metric::Cosine,
        }],
        Some(tk),
    )
    .expect("opts")
    .with_writer_pool(writer_pool)
}

/// Build one chunked RecordBatch slicing the cached corpus. Chunk
/// `i / n_chunks` covers `[i * chunk_size, (i+1) * chunk_size)`.
fn batch_chunk(corpus: &Corpus, chunk_idx: usize, n_chunks: usize) -> RecordBatch {
    let total = corpus.n;
    let chunk_size = total.div_ceil(n_chunks);
    let start = chunk_idx * chunk_size;
    let end = (start + chunk_size).min(total);
    let len = end - start;

    let titles = LargeStringArray::from(
        corpus.titles[start..end]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let vec_slice = &corpus.vectors[start * crate::corpus::DIM..end * crate::corpus::DIM];
    let item_field = Arc::new(Field::new("item", DataType::Float32, true));
    let values = Float32Array::from(vec_slice.to_vec());
    let fsl = FixedSizeListArray::try_new(
        item_field,
        crate::corpus::DIM as i32,
        Arc::new(values) as Arc<dyn Array>,
        None,
    )
    .expect("FSL");
    RecordBatch::try_new(
        supertable_schema(),
        vec![Arc::new(titles), Arc::new(fsl)],
    )
    .expect("batch")
    .slice(0, len)
}

/// Build a supertable end-to-end using the pre-built chunked
/// batches (see [`batches`]).
pub fn build_supertable(corpus: &Corpus, writer_threads: usize) -> Supertable {
    let st = Supertable::create(supertable_options(corpus.n_cent, writer_threads));
    let mut w = st.writer().expect("writer");
    for batch in batches() {
        w.append(batch).expect("append");
    }
    w.commit().expect("commit");
    drop(w);
    st
}

/// One pre-built supertable at the default writer-pool size,
/// reused across the query bench iterations.
static PREBUILT: OnceLock<Supertable> = OnceLock::new();

pub fn prebuilt_supertable() -> &'static Supertable {
    PREBUILT.get_or_init(|| build_supertable(corpus(), num_cpus::get().max(1)))
}
