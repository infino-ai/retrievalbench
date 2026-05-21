//! Stress-scale FTS recall: WAND+BMW must return the same top-k set
//! as Tantivy on overlap-heavy queries. Diagnostic helpers
//! (`diag_*`) print scores when investigating regressions; the body
//! of [`run`] enforces 20 pinned recall + ranking checks.
//!
//! Why this exists: at 60-doc scale (`against_tantivy.rs`) the heap
//! threshold never gets tight enough to exercise BMW's skip
//! decision. The recall bug that motivated this test only fires when
//!
//!   1. multiple cursors are positioned at the same `pivot_doc`
//!      (overlap-heavy queries — adjacent Zipfian ranks share many
//!      docs), AND
//!   2. the heap is filled with multi-term hits whose scores exceed
//!      any single block_max, so BMW UB at `pivot_doc` matters.
//!
//! Without the **pivot extension** step (extending the WAND pivot
//! prefix to include every cursor at `pivot_doc`), the BMW UB
//! undercounts and the search skips real candidates. The runner is
//! sized so condition (2) holds for the chosen queries, and it
//! detects the regression by comparing top-10 sets to Tantivy.
//!
//! Scale: 20K docs × 200 tokens (4M tokens) builds both engines in
//! ~5 s optimized. The bug manifests at this scale because k=10 and
//! Zipfian rank-50/51/52 produce enough triple-overlap docs to fill
//! the heap with 3-term hits. Runs in the bench-scale lane via
//! `cargo bench --bench scale -- fts_recall` so it gets the release
//! profile by default.

// diag_* helpers stay in the binary for manual invocation.
#![allow(dead_code)]

use bytes::Bytes;
use infino::superfile::fts::builder::FtsBuilder;
use infino::superfile::fts::reader::{BoolMode, FtsReader, OrAlgo};
use infino::test_helpers::default_tokenizer;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::collections::HashSet;
use std::sync::OnceLock;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{INDEXED, STORED, Schema as TSchema, TEXT};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{Index, IndexSettings, doc};

const N_DOCS: usize = 20_000;
const TOKENS_PER_DOC: usize = 200;
const VOCAB_SIZE: usize = 10_000;

fn build_zipf_cum(n: usize) -> Vec<f64> {
    let mut cum = Vec::with_capacity(n);
    let mut acc = 0.0f64;
    for i in 1..=n {
        acc += 1.0 / (i as f64);
        cum.push(acc);
    }
    cum
}

fn sample_zipf<R: rand::Rng>(cum: &[f64], rng: &mut R) -> usize {
    use rand::RngExt;
    let total = *cum.last().expect("non-empty");
    let target = rng.random::<f64>() * total;
    match cum.binary_search_by(|p| p.partial_cmp(&target).unwrap_or(std::cmp::Ordering::Equal)) {
        Ok(i) | Err(i) => i.min(cum.len() - 1) + 1,
    }
}

fn generate_corpus() -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(42);
    let zipf = build_zipf_cum(VOCAB_SIZE);
    let mut out = Vec::with_capacity(N_DOCS);
    for _ in 0..N_DOCS {
        let mut doc = String::with_capacity(TOKENS_PER_DOC * 8);
        for j in 0..TOKENS_PER_DOC {
            let r = sample_zipf(&zipf, &mut rng);
            if j > 0 {
                doc.push(' ');
            }
            doc.push_str(&format!("term{r:05}"));
        }
        out.push(doc);
    }
    out
}

fn build_infino(docs: &[String]) -> FtsReader {
    let mut b = FtsBuilder::new(default_tokenizer());
    b.register_column("title".to_string())
        .expect("register column");
    for (i, d) in docs.iter().enumerate() {
        b.add_doc(0, i as u32, d).expect("add doc");
    }
    let blob = b.finish();
    let json = r#"[{"name":"title","tokenizer":"ascii_lower"}]"#;
    FtsReader::open(Bytes::from(blob), json).expect("open FtsReader")
}

struct TantivyHandles {
    index: Index,
    title_field: tantivy::schema::Field,
}

fn build_tantivy(docs: &[String]) -> TantivyHandles {
    let mut sb = TSchema::builder();
    let _ = sb.add_u64_field("doc_id", INDEXED | STORED);
    let _ = sb.add_text_field("title", TEXT);
    let schema = sb.build();
    let index = Index::builder()
        .schema(schema.clone())
        .settings(IndexSettings::default())
        .create_in_ram()
        .expect("Tantivy create_in_ram");
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build();
    index.tokenizers().register("default", analyzer);
    let id_f = schema.get_field("doc_id").expect("get field");
    let title_f = schema.get_field("title").expect("get field");
    let mut writer = index.writer(100_000_000).expect("create Tantivy writer");
    for (i, d) in docs.iter().enumerate() {
        writer
            .add_document(doc!(id_f => i as u64, title_f => d.as_str()))
            .expect("add Tantivy document");
    }
    writer.commit().expect("commit builder");
    TantivyHandles {
        index,
        title_field: title_f,
    }
}

// Build once, reuse across tests. Each test runs queries against
// already-built indexes; the corpus + index build is the expensive
// part.
static CORPUS: OnceLock<Vec<String>> = OnceLock::new();
static INFINO: OnceLock<FtsReader> = OnceLock::new();
static TANTIVY: OnceLock<TantivyHandles> = OnceLock::new();

fn corpus() -> &'static [String] {
    CORPUS.get_or_init(generate_corpus).as_slice()
}
fn infino() -> &'static FtsReader {
    INFINO.get_or_init(|| build_infino(corpus()))
}
fn tantivy() -> &'static TantivyHandles {
    TANTIVY.get_or_init(|| build_tantivy(corpus()))
}

fn infino_top_k_scored(terms: &[&str], k: usize) -> Vec<(u32, f32)> {
    infino()
        .search("title", terms, k, BoolMode::Or)
        .expect("search")
}

fn tantivy_top_k_scored(query: &str, k: usize) -> Vec<(u32, f32)> {
    use tantivy::TantivyDocument;
    use tantivy::schema::Value;
    let h = tantivy();
    let r = h.index.reader().expect("open Tantivy reader");
    let s = r.searcher();
    let p = QueryParser::for_index(&h.index, vec![h.title_field]);
    let q = p.parse_query(query).expect("parse Tantivy query");
    let top = s
        .search(&q, &TopDocs::with_limit(k).order_by_score())
        .expect("search");
    let id_field = h.index.schema().get_field("doc_id").expect("get field");
    top.into_iter()
        .map(|(score, addr)| {
            let doc: TantivyDocument = s.doc(addr).expect("fetch tantivy doc");
            let id = doc
                .get_first(id_field)
                .expect("get first field")
                .as_u64()
                .expect("as u64") as u32;
            (id, score)
        })
        .collect()
}

fn infino_top_k(terms: &[&str], k: usize) -> Vec<u32> {
    infino_top_k_scored(terms, k)
        .into_iter()
        .map(|(d, _)| d)
        .collect()
}

fn tantivy_top_k(query: &str, k: usize) -> Vec<u32> {
    tantivy_top_k_scored(query, k)
        .into_iter()
        .map(|(d, _)| d)
        .collect()
}

/// Assert top-k *sets* agree. Order may diverge on ties because the
/// two engines tie-break differently, but the set membership is what
/// recall measures: missing a doc means BMW skipped a real candidate.
fn assert_top_k_set_agrees(terms: &[&str], query_str: &str, k: usize) {
    let inf: HashSet<u32> = infino_top_k(terms, k).into_iter().collect();
    let tan: HashSet<u32> = tantivy_top_k(query_str, k).into_iter().collect();
    assert_eq!(
        inf.len(),
        k,
        "infino returned {}/{} hits for {query_str:?}",
        inf.len(),
        k
    );
    assert_eq!(
        tan.len(),
        k,
        "tantivy returned {}/{} hits for {query_str:?}",
        tan.len(),
        k
    );
    if inf != tan {
        let missing: Vec<u32> = tan.difference(&inf).copied().collect();
        let extra: Vec<u32> = inf.difference(&tan).copied().collect();
        panic!(
            "top-{k} set mismatch on {query_str:?}\n  missing from infino (recall bug): {missing:?}\n  in infino but not tantivy: {extra:?}"
        );
    }
}

/// Tolerant set-equality: agreeing on docs that score *strictly above*
/// the kth-best score, allowing tie-zone divergence at exactly the
/// kth-score level. Bugs that hurt recall always pull a missed doc's
/// score *above* the kth-best (otherwise it wouldn't have made the
/// cut), so any divergence above the kth tier is a real bug. Below it
/// is just tie-break noise.
fn assert_top_k_recall(terms: &[&str], query_str: &str, k: usize) {
    let inf = infino_top_k_scored(terms, k);
    let tan = tantivy_top_k_scored(query_str, k);
    assert_eq!(
        inf.len(),
        k,
        "infino returned {}/{} hits for {query_str:?}",
        inf.len(),
        k
    );
    assert_eq!(
        tan.len(),
        k,
        "tantivy returned {}/{} hits for {query_str:?}",
        tan.len(),
        k
    );
    let inf_kth = inf.last().expect("non-empty").1;
    let tan_kth = tan.last().expect("non-empty").1;
    let kth_eps = inf_kth.max(tan_kth) * 1e-4;
    assert!(
        (inf_kth - tan_kth).abs() < kth_eps.max(1e-4),
        "kth-best scores disagree on {query_str:?}: infino={inf_kth}, tantivy={tan_kth}"
    );
    let inf_set: HashSet<u32> = inf.iter().map(|(d, _)| *d).collect();
    for (d, s) in &tan {
        if s > &(inf_kth + kth_eps.max(1e-4)) && !inf_set.contains(d) {
            panic!(
                "recall bug on {query_str:?}: tantivy doc {d} score={s} > infino kth ({inf_kth}) but missing from infino"
            );
        }
    }
}

/// Top-1 must be ranked the same. Top-1 is unique unless there's a
/// tied score, in which case the doc set tied at the top-1 score is
/// what we compare. Any disagreement here is a ranking bug — top-1
/// score is the most-strictly-bounded result either engine produces.
fn assert_top1_ranking_agrees(terms: &[&str], query_str: &str) {
    let inf = infino_top_k_scored(terms, 1);
    let tan = tantivy_top_k_scored(query_str, 1);
    assert!(
        !inf.is_empty() && !tan.is_empty(),
        "no hits for {query_str:?}"
    );
    let (inf_doc, inf_score) = inf[0];
    let (tan_doc, tan_score) = tan[0];
    let eps = inf_score.max(tan_score) * 1e-4;
    assert!(
        (inf_score - tan_score).abs() < eps.max(1e-4),
        "top-1 score disagrees on {query_str:?}: infino doc={inf_doc} score={inf_score}, tantivy doc={tan_doc} score={tan_score}"
    );
}

/// Diagnostic: prints scores from both engines for a query so a
/// failure can be inspected for "are these the same docs at the same
/// score, just tie-broken differently?" vs a real ranking divergence.

fn diag_three_similar_scores() {
    let inf = infino_top_k_scored(&["term00050", "term00051", "term00052"], 15);
    let tan = tantivy_top_k_scored("term00050 term00051 term00052", 15);
    println!("infino top-15:");
    for (d, s) in &inf {
        println!("  doc={d:>5}  score={s:.6}");
    }
    println!("tantivy top-15:");
    for (d, s) in &tan {
        println!("  doc={d:>5}  score={s:.6}");
    }
}

/// Compare WAND+BMW vs BMM (MaxScore) results for the same query. If
/// they disagree, one (or both) has an algorithm bug; if they agree
/// but both diverge from Tantivy, the bug is in shared infrastructure
/// (decode, scoring, doc-len lookup).

fn diag_three_similar_wand_vs_bmm() {
    let r = infino();
    let wand = r
        .search_with_algo_for_bench(
            "title",
            &["term00050", "term00051", "term00052"],
            15,
            OrAlgo::WandBmw,
        )
        .expect("WAND+BMW search");
    let bmm = r
        .search_with_algo_for_bench(
            "title",
            &["term00050", "term00051", "term00052"],
            15,
            OrAlgo::Bmm,
        )
        .expect("MaxScore+BMM search");
    println!("WAND+BMW top-15:");
    for (d, s) in &wand {
        println!("  doc={d:>5}  score={s:.6}");
    }
    println!("BMM top-15:");
    for (d, s) in &bmm {
        println!("  doc={d:>5}  score={s:.6}");
    }
}

/// Single-term sanity check: should match exactly. If this fails, the
/// recall divergence isn't WAND-related — it's tokenization / doc-len /
/// scoring formula.

fn diag_single_term_top10() {
    let inf = infino_top_k_scored(&["term00050"], 10);
    let tan = tantivy_top_k_scored("term00050", 10);
    println!("single term00050 — infino top-10:");
    for (d, s) in &inf {
        println!("  doc={d:>5}  score={s:.6}");
    }
    println!("single term00050 — tantivy top-10:");
    for (d, s) in &tan {
        println!("  doc={d:>5}  score={s:.6}");
    }
}

fn three_similar_recall_top10() {
    // Three adjacent Zipfian ranks: nearly identical IDF, postings
    // overlap on ~15-20% of all docs each. After the heap fills with
    // triple-overlap docs the threshold sits well above any single
    // cursor's block_max — the regime where pivot extension matters.
    assert_top_k_set_agrees(
        &["term00050", "term00051", "term00052"],
        "term00050 term00051 term00052",
        10,
    );
}

fn five_similar_recall_top10() {
    // Same overlap regime, deeper query. Five cursors mean even more
    // potential prefix configurations where pivot extension is the
    // difference between BMW skipping and scoring a real candidate.
    assert_top_k_set_agrees(
        &[
            "term00050",
            "term00051",
            "term00052",
            "term00053",
            "term00054",
        ],
        "term00050 term00051 term00052 term00053 term00054",
        10,
    );
}

fn three_wide_recall_top10() {
    // Wide UB-spread query (rank 1 + 50 + 100). Pivot stays at rank 1
    // most iterations; this query is more about confirming we don't
    // regress single-dominator correctness than stressing pivot
    // extension. Included so any future change to the WAND loop has a
    // recall guard for this shape too.
    assert_top_k_set_agrees(
        &["term00001", "term00050", "term00100"],
        "term00001 term00050 term00100",
        10,
    );
}

// ---- Varied-k recall ------------------------------------------------
//
// k changes the threshold dynamics: small k makes the heap fill quickly
// and threshold rise sharply (more BMW skip exercise); large k holds
// threshold low and forces more docs to be scored. Both shapes have to
// be correct.

fn three_similar_recall_k1() {
    // Top-1 must be a real top-1, with the same score Tantivy reports.
    assert_top1_ranking_agrees(
        &["term00050", "term00051", "term00052"],
        "term00050 term00051 term00052",
    );
}

fn three_similar_recall_k20() {
    // Larger k pushes the threshold tier deeper into the score
    // distribution, so the kth-best score changes shape — different
    // BMW skip dynamics than k=10.
    assert_top_k_recall(
        &["term00050", "term00051", "term00052"],
        "term00050 term00051 term00052",
        20,
    );
}

fn three_similar_recall_k50() {
    // 50 hits dwarfs the heap-fill phase; threshold spends most of the
    // run at the k=50 score floor. Catches bugs that only manifest
    // when the heap is "thoroughly populated."
    assert_top_k_recall(
        &["term00050", "term00051", "term00052"],
        "term00050 term00051 term00052",
        50,
    );
}

// ---- Single-term sanity --------------------------------------------
//
// Single-term goes through `search_single_term_bmw`, a different code
// path from `run_wand_bmw`. Worth its own coverage.

fn single_common_recall_k20() {
    // Long posting list (rank 1 — appears in most docs). BMW skip
    // gates almost every block; an off-by-one in the skip-table
    // logic shows up as missing top-k hits.
    assert_top_k_recall(&["term00001"], "term00001", 20);
}

fn single_rare_recall_top1() {
    // Tail-rank term — short posting list, often single-doc match.
    assert_top1_ranking_agrees(&["term09999"], "term09999");
}

// ---- Two-term overlap ----------------------------------------------

fn two_term_similar_recall_top10() {
    // Two adjacent ranks: maximal overlap, threshold settles where BMW
    // UB tightness matters most. A 2-term version of the original
    // recall regression.
    assert_top_k_recall(&["term00050", "term00051"], "term00050 term00051", 10);
}

fn two_term_dominator_recall_top10() {
    // One common term + one rare term — wide UB spread. Pivot is
    // almost always the common term's cursor.
    assert_top_k_recall(&["term00001", "term09000"], "term00001 term09000", 10);
}

// ---- Long queries --------------------------------------------------
//
// Pivot extension and alignment have an inner loop over the prefix;
// longer queries exercise more pivot-prefix configurations.

fn four_term_similar_recall_top10() {
    assert_top_k_recall(
        &["term00050", "term00051", "term00052", "term00053"],
        "term00050 term00051 term00052 term00053",
        10,
    );
}

fn six_term_similar_recall_top10() {
    assert_top_k_recall(
        &[
            "term00050",
            "term00051",
            "term00052",
            "term00053",
            "term00054",
            "term00055",
        ],
        "term00050 term00051 term00052 term00053 term00054 term00055",
        10,
    );
}

// ---- Mixed-magnitude UBs -------------------------------------------
//
// One very common term + several mid-tier terms. Pivot stays at the
// common term cursor most iterations; mid-tier suffix cursors cap
// the BMW skip target. If the suffix cap is wrong, mid-tier hits with
// the common term in their post body get missed.

fn dominator_plus_mid_tier_recall_top10() {
    assert_top_k_recall(
        &["term00001", "term00050", "term00051", "term00052"],
        "term00001 term00050 term00051 term00052",
        10,
    );
}

// ---- Asymmetric "long-tail" query ----------------------------------
//
// Mix of one common, one mid, one rare. The rare term has very high
// idf — its cursor's term_max dominates accum quickly so pivot_j
// often lands at the rare cursor. Tests the case where pivot_doc
// sits at the rare cursor's positions, which are sparse.

fn one_common_one_mid_one_rare_top10() {
    assert_top_k_recall(
        &["term00001", "term00050", "term05000"],
        "term00001 term00050 term05000",
        10,
    );
}

// ---- Top-1 ranking for every query shape ---------------------------
//
// Top-1 is the strictest ranking check (any tie-break ambiguity
// disappears with k=1 unless there's an exact-score tie at the head).
// If top-1 disagrees, scoring or skip logic has a real bug.

// ---- BMM (MaxScore) recall -----------------------------------------
//
// BMM is a separate algorithm path. The dispatcher may route here for
// similar-UB query shapes; recall must hold there too. We invoke BMM
// directly via the `search_with_algo_for_bench` test hook and compare
// against the same Tantivy oracle.

fn infino_bmm_top_k(terms: &[&str], k: usize) -> Vec<(u32, f32)> {
    infino()
        .search_with_algo_for_bench("title", terms, k, OrAlgo::Bmm)
        .expect("MaxScore+BMM search")
}

fn assert_bmm_top_k_recall(terms: &[&str], query_str: &str, k: usize) {
    let inf = infino_bmm_top_k(terms, k);
    let tan = tantivy_top_k_scored(query_str, k);
    assert_eq!(inf.len(), k, "infino-bmm hit count for {query_str:?}");
    assert_eq!(tan.len(), k, "tantivy hit count for {query_str:?}");
    let inf_kth = inf.last().expect("last element").1;
    let kth_eps = inf_kth.max(tan.last().expect("last element").1) * 1e-4;
    let inf_set: HashSet<u32> = inf.iter().map(|(d, _)| *d).collect();
    for (d, s) in &tan {
        if s > &(inf_kth + kth_eps.max(1e-4)) && !inf_set.contains(d) {
            panic!(
                "BMM recall bug on {query_str:?}: tantivy doc {d} score={s} > bmm kth ({inf_kth}) but missing"
            );
        }
    }
}

fn bmm_three_similar_recall_top10() {
    assert_bmm_top_k_recall(
        &["term00050", "term00051", "term00052"],
        "term00050 term00051 term00052",
        10,
    );
}

fn bmm_five_similar_recall_top10() {
    assert_bmm_top_k_recall(
        &[
            "term00050",
            "term00051",
            "term00052",
            "term00053",
            "term00054",
        ],
        "term00050 term00051 term00052 term00053 term00054",
        10,
    );
}

fn bmm_three_wide_recall_top10() {
    assert_bmm_top_k_recall(
        &["term00001", "term00050", "term00100"],
        "term00001 term00050 term00100",
        10,
    );
}

fn bmm_dominator_plus_mid_tier_recall_top10() {
    assert_bmm_top_k_recall(
        &["term00001", "term00050", "term00051", "term00052"],
        "term00001 term00050 term00051 term00052",
        10,
    );
}

fn bmm_six_term_similar_recall_top10() {
    assert_bmm_top_k_recall(
        &[
            "term00050",
            "term00051",
            "term00052",
            "term00053",
            "term00054",
            "term00055",
        ],
        "term00050 term00051 term00052 term00053 term00054 term00055",
        10,
    );
}

fn top1_ranking_battery() {
    let cases: &[(&[&str], &str)] = &[
        (&["term00001"], "term00001"),
        (&["term00050", "term00051"], "term00050 term00051"),
        (
            &["term00050", "term00051", "term00052"],
            "term00050 term00051 term00052",
        ),
        (
            &["term00001", "term00050", "term00100"],
            "term00001 term00050 term00100",
        ),
        (
            &[
                "term00050",
                "term00051",
                "term00052",
                "term00053",
                "term00054",
            ],
            "term00050 term00051 term00052 term00053 term00054",
        ),
    ];
    for (terms, query) in cases {
        assert_top1_ranking_agrees(terms, query);
    }
}

pub fn run() {
    println!("fts_recall: building corpus + indices (20K docs, Zipf-10K vocab)");
    // Force lazy-init of corpus + readers before the checks run, so
    // the first call doesn't include build cost in its assertions.
    let _ = corpus();
    let _ = infino();
    let _ = tantivy();

    println!("fts_recall: running 20 pinned-recall + ranking checks");
    three_similar_recall_top10();
    five_similar_recall_top10();
    three_wide_recall_top10();
    three_similar_recall_k1();
    three_similar_recall_k20();
    three_similar_recall_k50();
    single_common_recall_k20();
    single_rare_recall_top1();
    two_term_similar_recall_top10();
    two_term_dominator_recall_top10();
    four_term_similar_recall_top10();
    six_term_similar_recall_top10();
    dominator_plus_mid_tier_recall_top10();
    one_common_one_mid_one_rare_top10();
    bmm_three_similar_recall_top10();
    bmm_five_similar_recall_top10();
    bmm_three_wide_recall_top10();
    bmm_dominator_plus_mid_tier_recall_top10();
    bmm_six_term_similar_recall_top10();
    top1_ranking_battery();
    println!("fts_recall: all 20 pinned checks passed");
    println!(
        "fts_recall: (3 diagnostic fns remain in the binary — invoke directly to print scores)"
    );
}
