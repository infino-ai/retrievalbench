//! Negation (`-term`) three-way at supertable scale: infino
//! (multi-segment supertable) vs Tantivy (multi-segment disk index) vs
//! Lance FTS. Doc count defaults to 10M; override with
//! `INFINO_BENCH_DOC_COUNT` (chunk count stays 10).
//!
//! The corpus is generated and ingested in 1M-doc chunks so it never
//! lives in RAM whole (~20 GB as strings). Correctness parity was
//! proven at 1M (corpus-truth verify + Tantivy overlap); this run is
//! latency only — hit counts are still asserted.
//!
//! Run: `cargo bench --bench fts-negation-supertable`.

use std::sync::Arc;

use arrow_array::{LargeStringArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use tantivy::collector::TopDocs;
use tantivy::indexer::NoMergePolicy;
use tantivy::query::QueryParser;
use tantivy::schema::{INDEXED, STORED, Schema as TSchema, TEXT};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{Index, doc};
use tempfile::TempDir;

use infino::superfile::builder::FtsConfig;
use infino::superfile::fts::reader::BoolMode;
use infino::supertable::{Supertable, SupertableOptions};
use infino::test_helpers::default_tokenizer;

use retrievalbench::corpus::{generate_text_corpus, p50_micros};
use retrievalbench::lance;

fn n_docs() -> usize {
    std::env::var("INFINO_BENCH_DOC_COUNT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|&n: &usize| n > 0)
        .unwrap_or(10_000_000)
}
const N_CHUNKS: usize = 10;
const TOP_K: usize = 10;
const ITERS: usize = 20;
const LANCE_ITERS: usize = 10;

fn schema_title() -> Arc<Schema> {
    Arc::new(Schema::new(vec![Field::new(
        "title",
        DataType::LargeUtf8,
        false,
    )]))
}

pub fn run() {
    let n_docs = n_docs();
    let chunk_len = n_docs / N_CHUNKS;

    // infino: in-memory multi-segment supertable, one commit per chunk.
    let opts = SupertableOptions::new(
        schema_title(),
        vec![FtsConfig {
            column: "title".into(),
        }],
        vec![],
        Some(default_tokenizer()),
    )
    .expect("options");
    let st = Supertable::create(opts).expect("create");
    let mut w = st.writer().expect("writer");

    // Tantivy: disk index, analyzer matched to infino's tokenizer, one
    // segment per chunk (NoMergePolicy), like the existing 10M bench.
    let tan_dir = TempDir::new().expect("tantivy dir");
    let mut sb = TSchema::builder();
    sb.add_u64_field("doc_id", INDEXED | STORED);
    sb.add_text_field("title", TEXT);
    let tschema = sb.build();
    let tan = Index::create_in_dir(tan_dir.path(), tschema.clone()).expect("tantivy create");
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build();
    tan.tokenizers().register("default", analyzer);
    let tan_id = tschema.get_field("doc_id").expect("field");
    let tan_title = tschema.get_field("title").expect("field");
    let mut tw = tan.writer(200_000_000).expect("tantivy writer");
    tw.set_merge_policy(Box::new(NoMergePolicy));

    // Lance: chunked create/add, index built after the last chunk.
    let lance_dir = TempDir::new().expect("lance dir");
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut lance_table = None;

    for c in 0..N_CHUNKS {
        eprintln!("[fts-negation-supertable] chunk {}/{N_CHUNKS}: generate + ingest...", c + 1);
        let docs = generate_text_corpus(chunk_len, 100 + c as u64);
        let base = (c * chunk_len) as u32;

        let titles = LargeStringArray::from(docs.iter().map(String::as_str).collect::<Vec<_>>());
        let batch = RecordBatch::try_new(schema_title(), vec![Arc::new(titles)]).expect("batch");
        w.append(&batch).expect("append");
        w.commit().expect("commit");

        for (i, t) in docs.iter().enumerate() {
            tw.add_document(doc!(tan_id => (base as u64) + i as u64, tan_title => t.as_str()))
                .expect("tantivy add");
        }
        tw.commit().expect("tantivy commit");

        match &lance_table {
            None => {
                lance_table = Some(lance::lance_fts_table_chunked_begin(
                    &rt,
                    lance_dir.path(),
                    base,
                    &docs,
                ))
            }
            Some(t) => lance::lance_fts_table_chunked_add(&rt, t, base, &docs),
        }
        // chunk dropped here — peak RAM stays ~1 chunk + engine state.
    }
    eprintln!("[fts-negation-supertable] building lance FTS index...");
    let lance_table = lance_table.expect("lance table");
    lance::lance_fts_table_chunked_finish(&rt, &lance_table);

    let r = st.reader();
    let tan_reader = tan.reader().expect("tantivy reader");
    let tan_parser = QueryParser::for_index(&tan, vec![tan_title]);

    // (label, query string, infino mode, positive terms, negated term or "")
    let queries: &[(&str, &str, BoolMode, &[&str], &str)] = &[
        ("single_mid", "term00050", BoolMode::Or, &["term00050"], ""),
        ("two_term_or", "term00050 term00100", BoolMode::Or, &["term00050", "term00100"], ""),
        ("mid_pos_common_neg", "term00050 -term00005", BoolMode::Or, &["term00050"], "term00005"),
        ("mid_pos_rare_neg", "term00050 -term09000", BoolMode::Or, &["term00050"], "term09000"),
        (
            "two_mid_or_common_neg",
            "term00050 term00100 -term00005",
            BoolMode::Or,
            &["term00050", "term00100"],
            "term00005",
        ),
        (
            "two_mid_and_common_neg",
            "term00050 term00100 -term00005",
            BoolMode::And,
            &["term00050", "term00100"],
            "term00005",
        ),
    ];

    println!();
    println!(
        "{:<24} {:>12} {:>6} {:>12} {:>6} {:>12} {:>6}",
        "query", "infino p50", "hits", "tantivy p50", "hits", "lance p50", "hits"
    );
    println!("{}", "-".repeat(86));

    for (label, query, mode, positives, negated) in queries {
        let mut inf_hits = 0usize;
        let inf_us = p50_micros(
            || {
                let hits = r.bm25_search("title", query, TOP_K, *mode).expect("infino");
                inf_hits = hits.len();
            },
            ITERS,
        );

        // Bare terms parse as Should (Or); `+term` as Must (And).
        let tantivy_query = match mode {
            BoolMode::Or => query.to_string(),
            BoolMode::And => {
                let must: Vec<String> = positives.iter().map(|t| format!("+{t}")).collect();
                if negated.is_empty() {
                    must.join(" ")
                } else {
                    format!("{} -{negated}", must.join(" "))
                }
            }
        };
        let tan_q = tan_parser.parse_query(&tantivy_query).expect("tantivy parse");
        let searcher = tan_reader.searcher();
        let mut tan_hits = 0usize;
        let tan_us = p50_micros(
            || {
                let top = searcher
                    .search(&tan_q, &TopDocs::with_limit(TOP_K).order_by_score())
                    .expect("tantivy search");
                tan_hits = top.len();
            },
            ITERS,
        );

        let lance_q = match (negated.is_empty(), mode) {
            (true, _) => lance::make_lance_fts_or_query(query),
            (false, BoolMode::Or) => lance::make_lance_fts_not_query(positives, negated),
            (false, BoolMode::And) => lance::make_lance_fts_and_not_query(positives, negated),
        };
        let mut lance_hits = 0usize;
        let lance_us = p50_micros(
            || {
                lance_hits =
                    lance::bench_lance_fts_query(&rt, &lance_table, lance_q.clone(), TOP_K);
            },
            LANCE_ITERS,
        );

        println!(
            "{label:<24} {inf_us:>10.1}us {inf_hits:>6} {tan_us:>10.1}us {tan_hits:>6} {lance_us:>10.1}us {lance_hits:>6}"
        );
        assert_eq!(inf_hits, TOP_K, "{label}: infino under-filled");
        assert_eq!(tan_hits, TOP_K, "{label}: tantivy under-filled");
    }
    println!();
}
