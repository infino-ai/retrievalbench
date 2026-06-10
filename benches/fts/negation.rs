//! Negation (`-term`) three-way: infino superfile vs Tantivy vs Lance,
//! single segment, with a corpus-truth correctness check per query
//! (returned docs must match a positive term, never the negated term,
//! and overlap Tantivy's top-k where scores differentiate).
//!
//! A plain `main` (not criterion): each engine is built once on the
//! shared corpus, then each query is timed with `p50_micros`. All
//! engines get the same semantics — infino parses `-term`, Tantivy's
//! `QueryParser` maps it to `Occur::MustNot`, Lance gets a
//! `BooleanQuery` with a `MustNot` clause.
//!
//! Run: `cargo bench --bench fts-negation`.

use std::collections::HashSet;
use std::sync::Arc;

use arrow_array::{LargeStringArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use bytes::Bytes;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{INDEXED, STORED, Schema as TSchema, TEXT, Value};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{Index, IndexSettings, TantivyDocument, doc};
use tempfile::TempDir;

use infino::superfile::SuperfileReader;
use infino::superfile::builder::{BuilderOptions, FtsConfig, SuperfileBuilder};
use infino::superfile::fts::reader::BoolMode;
use infino::test_helpers::{decimal128_ids, default_tokenizer};
use retrievalbench::corpus::{generate_text_corpus, p50_micros};
use retrievalbench::lance;

/// Corpus size — matches the standard retrievalbench FTS bench scale.
const N_DOCS: usize = 1_000_000;
const SEED: u64 = 42;
const TOP_K: usize = 10;
/// Median over this many runs per query per engine.
const ITERS: usize = 50;

/// The query battery. Each is a `(label, positive terms, negated term)`
/// triple; the query string handed to both engines is
/// `"<positives> -<negated>"`. Terms are Zipfian `term00000`-style
/// tokens, so low indices are common and high indices are rare.
const QUERIES: &[(&str, &str, &str, BoolMode)] = &[
    // Mid-frequency positive (real idf → differentiated scores → a
    // well-defined top-k), common negated term (long negated posting
    // list — the case where streaming-exclude should beat decoding it
    // whole).
    ("mid_pos_common_neg", "term00050", "term00005", BoolMode::Or),
    // Mid-frequency positive, rare negated term (short negated list).
    ("mid_pos_rare_neg", "term00050", "term09000", BoolMode::Or),
    // Two-term OR positive (both mid-frequency), common negated term.
    ("two_mid_or_common_neg", "term00050 term00100", "term00005", BoolMode::Or),
    // Same two positives under And — both must match, negated dropped.
    (
        "two_mid_and_common_neg",
        "term00050 term00100",
        "term00005",
        BoolMode::And,
    ),
];

fn build_infino(docs: &[String]) -> SuperfileReader {
    // infino requires the id column to be Decimal128(38, 0).
    let schema = Arc::new(Schema::new(vec![
        Field::new("doc_id", DataType::Decimal128(38, 0), false),
        Field::new("title", DataType::LargeUtf8, false),
    ]));
    let opts = BuilderOptions::new(
        schema.clone(),
        "doc_id",
        vec![FtsConfig {
            column: "title".into(),
        }],
        vec![], // FTS only — no vector column.
        Some(default_tokenizer()),
    );
    let mut b = SuperfileBuilder::new(opts).expect("infino builder");
    let ids = decimal128_ids(0..docs.len() as u64);
    let titles = LargeStringArray::from(docs.iter().map(String::as_str).collect::<Vec<_>>());
    let batch = RecordBatch::try_new(schema, vec![Arc::new(ids), Arc::new(titles)]).expect("batch");
    b.add_batch(&batch, &[]).expect("add_batch");
    SuperfileReader::open(Bytes::from(b.finish().expect("finish"))).expect("open superfile")
}

fn build_tantivy(docs: &[String]) -> (Index, tantivy::schema::Field) {
    let mut sb = TSchema::builder();
    sb.add_u64_field("doc_id", INDEXED | STORED);
    sb.add_text_field("title", TEXT);
    let schema = sb.build();
    let index = Index::builder()
        .schema(schema.clone())
        .settings(IndexSettings::default())
        .create_in_ram()
        .expect("tantivy create_in_ram");
    // Match infino's AsciiLowerTokenizer: split on punctuation +
    // whitespace, lowercase, no stemming.
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build();
    index.tokenizers().register("default", analyzer);
    let id_f = schema.get_field("doc_id").expect("field");
    let title_f = schema.get_field("title").expect("field");
    let mut w = index.writer(100_000_000).expect("tantivy writer");
    for (i, text) in docs.iter().enumerate() {
        w.add_document(doc!(id_f => i as u64, title_f => text.as_str()))
            .expect("add doc");
    }
    w.commit().expect("commit");
    (index, title_f)
}

pub fn run() {
    eprintln!("[fts-negation] generating {N_DOCS}-doc corpus...");
    let docs = generate_text_corpus(N_DOCS, SEED);

    eprintln!("[fts-negation] building infino...");
    let infino = build_infino(&docs);
    eprintln!("[fts-negation] building tantivy...");
    let (tan, tan_title) = build_tantivy(&docs);
    let tan_reader = tan.reader().expect("tantivy reader");
    let tan_parser = QueryParser::for_index(&tan, vec![tan_title]);
    let tan_doc_id_field = tan.schema().get_field("doc_id").expect("doc_id field");

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    eprintln!("[fts-negation] building lance...");
    let lance_dir = TempDir::new().expect("tempdir");
    let (lance_table, _) = lance::build_lance_fts_table(&rt, lance_dir.path(), &docs, N_DOCS);

    println!();
    println!(
        "{:<24} {:>12} {:>6} {:>12} {:>6} {:>12} {:>6}",
        "query", "infino p50", "hits", "tantivy p50", "hits", "lance p50", "hits"
    );
    println!("{}", "-".repeat(86));

    for (label, positives, negated, mode) in QUERIES {
        let query = format!("{positives} -{negated}");
        let positive_terms: Vec<&str> = positives.split_whitespace().collect();
        // Tantivy's QueryParser: bare terms are Should (Or); `+term`
        // is Must (And). infino takes the same string + the mode.
        let tantivy_query = match mode {
            BoolMode::Or => query.clone(),
            BoolMode::And => {
                let must: Vec<String> =
                    positive_terms.iter().map(|t| format!("+{t}")).collect();
                format!("{} -{negated}", must.join(" "))
            }
        };

        // infino — bm25_search is async; block on the shared runtime.
        let mut infino_hits = 0usize;
        let infino_us = p50_micros(
            || {
                let hits = rt
                    .block_on(infino.bm25_search("title", &query, TOP_K, *mode))
                    .expect("infino bm25_search");
                infino_hits = hits.len();
            },
            ITERS,
        );

        // Tantivy — QueryParser turns `-term` into Occur::MustNot.
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

        // Lance — BooleanQuery with Should(positives) + MustNot(negated).
        let lance_q = match mode {
            BoolMode::Or => lance::make_lance_fts_not_query(&positive_terms, negated),
            BoolMode::And => lance::make_lance_fts_and_not_query(&positive_terms, negated),
        };
        let lance_hits = lance::search_lance_fts_query(&rt, &lance_table, lance_q.clone(), TOP_K).len();
        let lance_us = p50_micros(
            || {
                let n = lance::bench_lance_fts_query(&rt, &lance_table, lance_q.clone(), TOP_K);
                std::hint::black_box(n);
            },
            ITERS,
        );

        println!(
            "{label:<24} {infino_us:>10.1}us {infino_hits:>6} {tan_us:>10.1}us {tan_hits:>6} {lance_us:>10.1}us {lance_hits:>6}"
        );

        // ---- Correctness check on infino's returned docs ----------
        // (a) every returned doc must contain a positive term,
        // (b) none may contain the negated term,
        // (c) the result set should overlap Tantivy's (the reference).
        let infino_top: Vec<(u32, f32)> = rt
            .block_on(infino.bm25_search("title", &query, TOP_K, BoolMode::Or))
            .expect("infino verify");
        let infino_ids: Vec<u32> = infino_top.iter().map(|(d, _)| *d).collect();
        // Score spread of the returned top-k: when min≈max, the whole
        // top-k is a tied-score pool, so which exact docs surface is a
        // tie-break choice (explains low overlap vs another engine).
        let score_min = infino_top.iter().map(|(_, s)| *s).fold(f32::INFINITY, f32::min);
        let score_max = infino_top.iter().map(|(_, s)| *s).fold(f32::NEG_INFINITY, f32::max);
        let n = infino_ids.len();
        let relevant = infino_ids
            .iter()
            .filter(|&&id| {
                let toks: HashSet<&str> = docs[id as usize].split_whitespace().collect();
                match mode {
                    BoolMode::Or => positive_terms.iter().any(|p| toks.contains(p)),
                    BoolMode::And => positive_terms.iter().all(|p| toks.contains(p)),
                }
            })
            .count();
        let clean = infino_ids
            .iter()
            .filter(|&&id| !docs[id as usize].split_whitespace().any(|w| w == *negated))
            .count();
        let tan_ids: HashSet<u32> = searcher
            .search(&tan_q, &TopDocs::with_limit(TOP_K).order_by_score())
            .expect("tantivy verify")
            .into_iter()
            .map(|(_, addr)| {
                let d: TantivyDocument = searcher.doc(addr).expect("fetch doc");
                d.get_first(tan_doc_id_field)
                    .and_then(|v| v.as_u64())
                    .expect("doc_id") as u32
            })
            .collect();
        let overlap = infino_ids.iter().filter(|id| tan_ids.contains(id)).count();
        println!(
            "    verify: relevant {relevant}/{n}  exclude-clean {clean}/{n}  overlap-vs-tantivy {overlap}/{n}  top-k score [{score_min:.4}..{score_max:.4}]"
        );
    }
    println!();

}
