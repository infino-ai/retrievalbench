//! Tantivy head-to-head FTS bench for the superfile layer.
//!
//! Measures Tantivy only — infino's own numbers come from
//! `infino/benches` (read out of `../infino/target/criterion/...` at
//! emit time). The corpus, queries, and ground truth are shared
//! between both repos via `infino::test_helpers::bench_corpus`
//! (re-exported here as `crate::corpus`), so the comparison rows in
//! the rendered README are apples-to-apples.
//!
//! Pinned to 1M-doc Zipfian (200 tokens/doc, 10K vocab). The
//! single-superfile shape is rarely much larger in production —
//! `benches/fts/supertable.rs` covers the 10M+ supertable scale.
//!
//! ## Invocation
//!
//! ```text
//! cargo bench --bench fts                            # all FTS
//! cargo bench --bench fts -- superfile_fts_build     # ingest only
//! cargo bench --bench fts -- superfile_fts_search    # search only
//! ```
//!
//! ## Workflow
//!
//! Run infino's bench first so its criterion output is on disk:
//!
//! ```text
//! (cd ../infino && cargo bench --bench fts -- superfile_fts)
//! cargo bench --bench fts -- superfile_fts
//! ```
//!
//! The comparison column shows "—" for infino if infino's criterion
//! output is missing.

use criterion::{BenchmarkGroup, Criterion, Throughput, criterion_group, measurement::WallTime};
use std::hint::black_box;
use std::sync::OnceLock;
use tantivy::Index;
use tantivy::collector::TopDocs;
use tantivy::doc;
use tantivy::query::{Query, QueryParser};
use tantivy::schema::{
    INDEXED, IndexRecordOption, STORED, Schema, TextFieldIndexing, TextOptions,
};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};

// ─── Constants ────────────────────────────────────────────────────────

/// Tantivy heap budget for the indexer at 1M docs.
const TANTIVY_HEAP_BYTES: usize = 500_000_000;

/// Doc count. Pinned to 1M — the supertable shape is what scales out.
const N_DOCS: usize = 1_000_000;

enum TantivyThreads {
    Single,
    Default,
}

struct TantivyHandles {
    index: Index,
    title_field: tantivy::schema::Field,
}

// ─── Fixtures ────────────────────────────────────────────────────────

static DOCS: OnceLock<Vec<String>> = OnceLock::new();
static TANTIVY: OnceLock<TantivyHandles> = OnceLock::new();

fn docs() -> &'static [String] {
    DOCS.get_or_init(|| crate::corpus::generate_text_corpus(N_DOCS, 1))
        .as_slice()
}

fn tantivy_handles() -> &'static TantivyHandles {
    TANTIVY.get_or_init(|| build_tantivy(docs(), TantivyThreads::Single))
}

// ─── Tantivy builder + search ─────────────────────────────────────────

fn build_tantivy(docs: &[String], threads: TantivyThreads) -> TantivyHandles {
    let mut sb = Schema::builder();
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

    let mut writer = match threads {
        TantivyThreads::Single => index
            .writer_with_num_threads(1, TANTIVY_HEAP_BYTES)
            .expect("writer"),
        TantivyThreads::Default => index.writer(TANTIVY_HEAP_BYTES).expect("writer"),
    };

    for (i, t) in docs.iter().enumerate() {
        writer
            .add_document(doc!(
                id_field => i as u64,
                title_field => t.as_str(),
            ))
            .expect("add_document");
    }
    writer.commit().expect("commit");
    drop(writer);
    TantivyHandles { index, title_field }
}

fn tantivy_search_scored(handles: &TantivyHandles, q: &dyn Query, k: usize) -> Vec<(u32, f32)> {
    let reader = handles.index.reader().expect("reader");
    let searcher = reader.searcher();
    let top = searcher
        .search(q, &TopDocs::with_limit(k).order_by_score())
        .expect("search");
    top.into_iter()
        .map(|(score, addr)| (addr.doc_id, score))
        .collect()
}

// ─── Bench helpers ────────────────────────────────────────────────────

fn bench_tantivy_only(
    g: &mut BenchmarkGroup<WallTime>,
    name: &str,
    t: &TantivyHandles,
    tantivy_query: &dyn Query,
) {
    g.bench_function(format!("{name}_tantivy_top10"), |b| {
        b.iter(|| {
            let hits = tantivy_search_scored(t, tantivy_query, black_box(10));
            black_box(hits)
        });
    });
}

// ─── Bench entry ──────────────────────────────────────────────────────

fn bench(c: &mut Criterion) {
    eprintln!("[fts/superfile] building Tantivy ({N_DOCS} docs)...");
    let t = tantivy_handles();
    eprintln!("[fts/superfile] Tantivy ready");

    // ---- Ingest sub-bench (group: superfile_fts_build) -------------
    {
        let n = N_DOCS;
        let docs_for_ingest = docs();
        let mut g = c.benchmark_group("superfile_fts_build");
        g.sample_size(10);
        g.throughput(Throughput::Elements(n as u64));

        g.bench_function(format!("tantivy_1thread_{n}docs"), |b| {
            b.iter_with_large_drop(|| {
                build_tantivy(black_box(docs_for_ingest), TantivyThreads::Single)
            });
        });
        g.bench_function(format!("tantivy_default_threads_{n}docs"), |b| {
            b.iter_with_large_drop(|| {
                build_tantivy(black_box(docs_for_ingest), TantivyThreads::Default)
            });
        });
        g.finish();

        emit_ingest_markdown();
    }

    // ---- Search sub-bench (group: superfile_fts_search) ------------
    {
        let parser = QueryParser::for_index(&t.index, vec![t.title_field]);
        let q_single_rare = parser.parse_query("term09999").expect("parse");
        let q_single_df1 = parser.parse_query("doc0500000").expect("parse");
        let q_single_common = parser.parse_query("term00001").expect("parse");
        let q_two = parser.parse_query("term00001 term00050").expect("parse");
        let q_three_wide = parser
            .parse_query("term00001 term00050 term00100")
            .expect("parse");
        let q_three_similar = parser
            .parse_query("term00050 term00051 term00052")
            .expect("parse");
        let q_five = parser
            .parse_query("term00050 term00051 term00052 term00053 term00054")
            .expect("parse");
        let q_two_and = parser.parse_query("+term00001 +term00050").expect("parse");
        let q_three_wide_and = parser
            .parse_query("+term00001 +term00050 +term00100")
            .expect("parse");
        let q_three_similar_and = parser
            .parse_query("+term00050 +term00051 +term00052")
            .expect("parse");
        let q_five_and = parser
            .parse_query("+term00050 +term00051 +term00052 +term00053 +term00054")
            .expect("parse");

        let mut g = c.benchmark_group("superfile_fts_search");

        bench_tantivy_only(&mut g, "single_rare", t, q_single_rare.as_ref());
        bench_tantivy_only(&mut g, "single_df1", t, q_single_df1.as_ref());
        bench_tantivy_only(&mut g, "single_common", t, q_single_common.as_ref());
        bench_tantivy_only(&mut g, "two_term_or", t, q_two.as_ref());
        bench_tantivy_only(&mut g, "three_wide", t, q_three_wide.as_ref());
        bench_tantivy_only(&mut g, "three_similar", t, q_three_similar.as_ref());
        bench_tantivy_only(&mut g, "five_term", t, q_five.as_ref());
        bench_tantivy_only(&mut g, "two_term_and", t, q_two_and.as_ref());
        bench_tantivy_only(&mut g, "three_wide_and", t, q_three_wide_and.as_ref());
        bench_tantivy_only(&mut g, "three_similar_and", t, q_three_similar_and.as_ref());
        bench_tantivy_only(&mut g, "five_term_and", t, q_five_and.as_ref());

        g.finish();

        emit_search_markdown();
    }
}

// ─── Markdown summary emitters ────────────────────────────────────────

fn emit_ingest_markdown() {
    use crate::markdown::{
        MarkdownSection, fmt_throughput, fmt_time, fmt_winner, read_infino_mean_ns, read_mean_ns,
    };

    let mut body = String::new();
    body.push_str(&format!(
        "### Superfile FTS — ingest ({N_DOCS} docs, Zipfian, 200 tokens/doc, 10K vocab)\n\n"
    ));
    body.push_str("| Engine                       | Time       | Throughput | vs Tantivy        |\n");
    body.push_str("|------------------------------|------------|------------|-------------------|\n");

    let group = "superfile_fts_build";
    // Infino numbers come from infino's own bench harness.
    let infino_1t = read_infino_mean_ns(group, &format!("infino_1thread_{N_DOCS}docs"));
    let infino_rayon =
        read_infino_mean_ns(group, &format!("infino_rayon_default_threads_{N_DOCS}docs"));
    // Tantivy numbers come from this bench's own criterion output.
    let tantivy_1t = read_mean_ns(group, &format!("tantivy_1thread_{N_DOCS}docs"));
    let tantivy_def = read_mean_ns(group, &format!("tantivy_default_threads_{N_DOCS}docs"));

    let row = |label: &str, ns: Option<f64>, baseline: Option<f64>, is_baseline: bool| -> String {
        let time = ns.map(fmt_time).unwrap_or_else(|| "—".into());
        let thrpt = ns
            .map(|n| fmt_throughput((N_DOCS as f64) / (n / 1e9)))
            .unwrap_or_else(|| "—".into());
        let cmp = if is_baseline {
            "—".to_string()
        } else {
            fmt_winner("infino", ns, "tantivy", baseline)
        };
        format!("| {label:28} | {time:10} | {thrpt:10} | {cmp:17} |\n")
    };

    body.push_str(&row("infino_1thread", infino_1t, tantivy_1t, false));
    body.push_str(&row("tantivy_1thread", tantivy_1t, tantivy_1t, true));
    body.push_str(&row(
        "infino_rayon_default_threads",
        infino_rayon,
        tantivy_def,
        false,
    ));
    body.push_str(&row("tantivy_default_threads", tantivy_def, tantivy_def, true));

    crate::markdown::emit(&MarkdownSection {
        anchor_id: "bench/fts/superfile/ingest".into(),
        body,
    });
}

fn emit_search_markdown() {
    use crate::markdown::{MarkdownSection, fmt_time, fmt_winner, read_infino_mean_ns, read_mean_ns};

    let mut body = String::new();
    body.push_str(&format!("### Superfile FTS — search ({N_DOCS} docs)\n\n"));
    body.push_str("| Query          | infino     | Tantivy    | Winner                |\n");
    body.push_str("|----------------|------------|------------|-----------------------|\n");

    let group = "superfile_fts_search";
    let queries_or = [
        "single_rare",
        "single_df1",
        "single_common",
        "two_term_or",
        "three_wide",
        "three_similar",
        "five_term",
    ];
    let queries_and = [
        "two_term_and",
        "three_wide_and",
        "three_similar_and",
        "five_term_and",
    ];

    body.push_str("**OR queries:**\n\n");
    for q in queries_or {
        let inf = read_infino_mean_ns(group, &format!("{q}_infino_top10"));
        let tan = read_mean_ns(group, &format!("{q}_tantivy_top10"));
        let inf_s = inf.map(fmt_time).unwrap_or_else(|| "—".into());
        let tan_s = tan.map(fmt_time).unwrap_or_else(|| "—".into());
        let w = fmt_winner("infino", inf, "tantivy", tan);
        body.push_str(&format!("| {q:14} | {inf_s:10} | {tan_s:10} | {w:21} |\n"));
    }

    body.push_str("\n**AND queries:**\n\n");
    for q in queries_and {
        let inf = read_infino_mean_ns(group, &format!("{q}_infino_top10"));
        let tan = read_mean_ns(group, &format!("{q}_tantivy_top10"));
        let inf_s = inf.map(fmt_time).unwrap_or_else(|| "—".into());
        let tan_s = tan.map(fmt_time).unwrap_or_else(|| "—".into());
        let w = fmt_winner("infino", inf, "tantivy", tan);
        body.push_str(&format!("| {q:14} | {inf_s:10} | {tan_s:10} | {w:21} |\n"));
    }

    body.push('\n');
    body.push_str("**Per-algorithm probes** (infino-only, WAND+BMW vs MaxScore+BMM):\n\n");
    body.push_str("| Shape         | WAND+BMW   | MaxScore+BMM | Winner                |\n");
    body.push_str("|---------------|------------|--------------|-----------------------|\n");
    for shape in ["wide_3", "similar_3", "similar_5"] {
        let wand = read_infino_mean_ns(group, &format!("{shape}_wand_top10"));
        let bmm = read_infino_mean_ns(group, &format!("{shape}_bmm_top10"));
        let wand_s = wand.map(fmt_time).unwrap_or_else(|| "—".into());
        let bmm_s = bmm.map(fmt_time).unwrap_or_else(|| "—".into());
        let w = fmt_winner("WAND+BMW", wand, "MaxScore+BMM", bmm);
        body.push_str(&format!("| {shape:13} | {wand_s:10} | {bmm_s:12} | {w:21} |\n"));
    }

    crate::markdown::emit(&MarkdownSection {
        anchor_id: "bench/fts/superfile/search".into(),
        body,
    });
}

criterion_group!(benches, bench);
