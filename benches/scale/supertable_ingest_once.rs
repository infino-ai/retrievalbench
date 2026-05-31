//! Single-iteration FTS supertable ingest head-to-head vs Tantivy.
//!
//! Mirrors `benches/fts/supertable/build.rs` shape (10M-doc Zipfian
//! corpus, both engines at their auto thread budgets — infino's
//! `cpus/2` writer pool, Tantivy's heap-derived 8-thread cap) but
//! runs **one** build per engine instead of criterion's minimum 10
//! samples × 1 iter. Used when you want a single fresh number and
//! don't want to wait ~10–15 minutes for criterion to finish a full
//! statistical sample.
//!
//! Invocation:
//!
//! ```text
//! cargo bench --features bench-diagnostics --bench scale -- supertable_ingest_once
//! ```

use std::sync::Arc;
use std::time::Instant;

use arrow_array::{LargeStringArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use infino::config::Config;
use infino::superfile::builder::FtsConfig;
use infino::superfile::fts::tokenize::Tokenizer;
use infino::supertable::{Supertable, SupertableOptions};
use infino::test_helpers::default_tokenizer;
use tantivy::indexer::NoMergePolicy;
use tantivy::schema::{
    INDEXED, IndexRecordOption, STORED, Schema as TSchema, TextFieldIndexing, TextOptions,
};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{Index, doc};

use retrievalbench::corpus;

/// Doc count for the single-run ingest measurement. Matches the
/// supertable FTS bench's `N_DOCS` so the numbers are directly
/// comparable.
const N_DOCS: usize = 10_000_000;
/// Number of append batches before the single `commit()`. Infino
/// row-shards inside `commit()` regardless of this, so the
/// `SEGMENTS` knob no longer controls output segment count — kept
/// only to drive Tantivy's per-chunk commit cycle (it emits one
/// segment per `commit()` with `NoMergePolicy`).
const N_INPUT_CHUNKS: usize = 4;
/// Tantivy heap budget. Matches the supertable FTS bench; large
/// enough that the writer settles on the 8-thread cap, not a
/// heap-thrashing smaller count.
const TANTIVY_HEAP_BYTES: usize = 2_000_000_000;

fn schema_id_title() -> Arc<Schema> {
    Arc::new(Schema::new(vec![Field::new(
        "title",
        DataType::LargeUtf8,
        false,
    )]))
}

fn build_supertable_once(docs: &[String]) -> (Supertable, std::time::Duration) {
    let tk: Arc<dyn Tokenizer> = default_tokenizer();
    // Resolve the writer-pool size via the standard config stack:
    // `INFINO_SUPERTABLE__WRITER_THREADS=N` env var overrides the
    // `cpus/2` auto default. `commit()` row-shards into
    // `min(writer_pool.threads, total_rows)` superfiles, so this
    // knob doubles as the per-commit output-cardinality dial.
    // Examples:
    //   INFINO_SUPERTABLE__WRITER_THREADS=32 cargo bench --features bench-diagnostics --bench scale -- supertable_ingest_once
    //     → 32 superfiles (matches Tantivy's 8 internal threads × 4 commits = 32 segments)
    //   (unset)
    //     → cpus/2 superfiles (8 on a 16-core box, the default policy)
    let cfg = Config::load().expect("load infino config");
    let opts = SupertableOptions::new(
        schema_id_title(),
        vec![FtsConfig {
            column: "title".into(),
        }],
        vec![],
        Some(tk),
    )
    .expect("opts")
    .apply_config(&cfg)
    .expect("apply_config")
    .with_commit_threshold_size_mb(0); // auto-flush off; bench buffers everything

    let st = Supertable::create(opts);
    let mut w = st.writer().expect("writer");
    let chunk_size = docs.len().div_ceil(N_INPUT_CHUNKS);
    let schema = schema_id_title();

    let t0 = Instant::now();
    for chunk in docs.chunks(chunk_size) {
        let titles = LargeStringArray::from(chunk.iter().map(String::as_str).collect::<Vec<_>>());
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(titles)]).expect("batch");
        w.append(&batch).expect("append");
    }
    w.commit().expect("commit");
    drop(w);
    let elapsed = t0.elapsed();
    (st, elapsed)
}

fn build_tantivy_once(docs: &[String]) -> (Index, std::time::Duration) {
    let mut sb = TSchema::builder();
    let id_field = sb.add_u64_field("doc_id", INDEXED | STORED);
    let title_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(IndexRecordOption::WithFreqs),
    );
    let title_field = sb.add_text_field("title", title_opts);
    let schema = sb.build();
    let index = Index::builder()
        .schema(schema)
        .create_in_ram()
        .expect("create_in_ram");
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build();
    index.tokenizers().register("default", analyzer);

    let mut writer = index.writer(TANTIVY_HEAP_BYTES).expect("writer");
    writer.set_merge_policy(Box::new(NoMergePolicy));

    let chunk_size = docs.len().div_ceil(N_INPUT_CHUNKS);
    let t0 = Instant::now();
    for (chunk_idx, chunk) in docs.chunks(chunk_size).enumerate() {
        let start = chunk_idx * chunk_size;
        for (i, t) in chunk.iter().enumerate() {
            writer
                .add_document(doc!(
                    id_field => (start + i) as u64,
                    title_field => t.as_str(),
                ))
                .expect("add_document");
        }
        writer.commit().expect("commit");
    }
    drop(writer);
    let elapsed = t0.elapsed();
    (index, elapsed)
}

pub fn run() {
    println!(
        "supertable_ingest_once: {} docs × Zipfian (200 tokens/doc, 10K vocab), 1 build per engine",
        N_DOCS
    );

    let t_corpus = Instant::now();
    let docs = corpus::generate_text_corpus(N_DOCS, 1);
    println!(
        "  corpus generated in {:.2} s",
        t_corpus.elapsed().as_secs_f64()
    );

    let (st, t_infino) = build_supertable_once(&docs);
    let n_superfiles = st.reader().n_superfiles();
    println!(
        "  infino: {:.2} s ({:.1} K docs/s)  → {} superfiles",
        t_infino.as_secs_f64(),
        (N_DOCS as f64) / t_infino.as_secs_f64() / 1000.0,
        n_superfiles,
    );
    drop(st);

    let (idx, t_tantivy) = build_tantivy_once(&docs);
    let n_tantivy_segments = idx.searchable_segments().map(|v| v.len()).unwrap_or(0);
    println!(
        "  tantivy: {:.2} s ({:.1} K docs/s)  → {} segments",
        t_tantivy.as_secs_f64(),
        (N_DOCS as f64) / t_tantivy.as_secs_f64() / 1000.0,
        n_tantivy_segments,
    );
    drop(idx);

    let ratio = t_infino.as_secs_f64() / t_tantivy.as_secs_f64();
    if ratio > 1.0 {
        println!("  ratio: infino {:.2}× slower than Tantivy", ratio,);
    } else {
        println!("  ratio: infino {:.2}× faster than Tantivy", 1.0 / ratio,);
    }
}
