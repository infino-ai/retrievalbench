//! Shared bench helpers: deterministic corpus generators, scale knob,
//! and pre-baked builders. Included as `mod common;` from each
//! benchmark binary (criterion benches are independent crates, so the
//! helpers can't live in the main library — they'd otherwise be
//! `pub fn`s leaking generator code into the public API).
//!
//! Scale strategy:
//!
//!   - Default: 1M docs. Runs in single-digit seconds per build bench
//!     and 1.5 GB peak RAM at dim=384, fits comfortably on a 16 GB
//!     dev laptop.
//!   - `INFINO_BENCH_FULL=1`: 10M docs. The plan's target scale.
//!     Vector at 10M × 384 (f32) = 14.6 GB — needs a 32 GB+ machine.
//!     Use this for milestone validation; baseline numbers in
//!     `benches/README.md` cover both scales.

#![allow(dead_code)] // Each bench uses a subset; deny would force per-bench cfg gates.

use bytes::Bytes;
use infino::superfile::SuperfileReader;
use infino::superfile::builder::{
    BuilderOptions, FtsConfig, SuperfileBuilder, VectorConfig as SfVectorConfig,
};
use infino::superfile::fts::builder::FtsBuilder;
use infino::test_helpers::default_tokenizer;
use infino::superfile::vector::builder::{VectorBuilder, VectorConfig};
use infino::superfile::vector::distance::{Metric, normalize};
use infino::superfile::vector::reader::VectorReader;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand_distr::{Distribution, StandardNormal};
use std::sync::Arc;

/// Resolved doc count for the current bench run.
pub fn n_docs() -> usize {
    if std::env::var("INFINO_BENCH_FULL").is_ok() {
        10_000_000
    } else {
        1_000_000
    }
}

/// Tokens per doc — chosen to land in the same magnitude as a typical
/// short article (~200 words). The product `n_docs * tokens_per_doc`
/// drives FTS posting volume.
pub const TOKENS_PER_DOC: usize = 200;

/// Vocabulary size — controls term-frequency distribution. Small enough
/// that common terms appear in many docs (exercising long posting
/// lists); large enough that rare terms exist (exercising the FST
/// + skip-table cold path).
pub const VOCAB_SIZE: usize = 10_000;

/// Vector dimension — matches modern sentence-embedding models
/// (all-MiniLM-L6-v2 = 384, BGE-small = 384).
pub const DIM: usize = 384;

/// IVF cluster count — `~sqrt(n_docs)` is the conventional setting.
/// We round to a fixed value so different-scale runs share the same
/// `n_cent` (cluster size scales with corpus size, which is what
/// matters for IVF behavior).
pub fn n_cent(n_docs: usize) -> usize {
    // 1M → 1024 clusters (~977 docs each)
    // 10M → 4096 clusters (~2441 docs each)
    if n_docs >= 5_000_000 {
        4096
    } else if n_docs >= 100_000 {
        1024
    } else {
        64
    }
}

/// Generate a Zipfian-distributed token corpus. Returns a flat
/// Returns a `Vec<String>` of length `n_docs`, each entry one document
/// containing `TOKENS_PER_DOC` body tokens plus one doc-unique
/// identifier token (`doc<7-digit-id>`).
///
/// Word frequency on the body follows Zipf's law (`f(rank) ∝ 1/rank`)
/// over a closed [`VOCAB_SIZE`] vocabulary, so a few terms dominate
/// (yielding long posting lists) and tail terms appear in few docs
/// (yielding short lists). This exercises BlockMaxWAND skipping, FST
/// tail merging, and the posting-codec's bit-width adaptation.
///
/// The per-doc identifier models the universal source of `df=1` terms
/// in production FTS: every real document carries some token unique to
/// itself (URL hash, ISBN, primary key, headline number). With a
/// closed-vocab Zipf body alone the corpus has no singletons — the
/// rarest body term still has df ≈ N / (V · H_V) ≈ 2000 at 1 M docs ×
/// 200 tokens × 10 K vocab — which underexercises the format's
/// singleton path (per-term metadata header pressure, BMW per-term
/// upper bound for one-doc terms, and the inline-encoding short-circuit
/// the builder applies when `df = 1`). Adding one doc-unique token per
/// doc creates a natural singleton long tail proportional to `n_docs`.
pub fn generate_text_corpus(n_docs: usize, seed: u64) -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let zipf = ZipfDistribution::new(VOCAB_SIZE);
    let mut out = Vec::with_capacity(n_docs);
    for doc_id in 0..n_docs {
        // +1 token slot for the doc-unique identifier prefix.
        let mut doc = String::with_capacity((TOKENS_PER_DOC + 1) * 8);
        // Doc-unique identifier — `df = 1` by construction.
        doc.push_str(&format!("doc{doc_id:07}"));
        for _ in 0..TOKENS_PER_DOC {
            let idx = zipf.sample(&mut rng);
            doc.push(' ');
            doc.push_str(&format!("term{idx:05}"));
        }
        out.push(doc);
    }
    out
}

/// Deterministic Zipfian sampler over `[1, n]`. Inverse-CDF sampler;
/// O(log n) per draw. Replaces `rand_distr::Zipf` to avoid pulling
/// the `f64` parameter overhead — we just want integer ranks.
pub struct ZipfDistribution {
    /// Cumulative `1/i` weights up to rank `n`. Index 0 == rank 1.
    cum_weights: Vec<f64>,
}

impl ZipfDistribution {
    pub fn new(n: usize) -> Self {
        let mut cum = Vec::with_capacity(n);
        let mut acc = 0.0f64;
        for i in 1..=n {
            acc += 1.0 / (i as f64);
            cum.push(acc);
        }
        Self { cum_weights: cum }
    }

    pub fn sample<R: rand::Rng>(&self, rng: &mut R) -> usize {
        use rand::RngExt;
        let total = *self.cum_weights.last().expect("non-empty");
        let target = rng.random::<f64>() * total;
        // Binary search; index of first weight ≥ target = rank-1.
        match self
            .cum_weights
            .binary_search_by(|p| p.partial_cmp(&target).unwrap_or(std::cmp::Ordering::Equal))
        {
            Ok(i) | Err(i) => i.min(self.cum_weights.len() - 1) + 1,
        }
    }
}

/// Generate `n_docs` planted-cluster vectors of `DIM` dimensions,
/// optionally per-doc normalized for cosine. `n_cent` planted
/// centers; each doc lives near a center with `sigma = 0.3` per-dim
/// Gaussian noise.
///
/// **Centers are intentionally NOT normalized.** Centers are drawn
/// from `3·N(0, 1)` per dim, giving `||c|| ≈ 3·√DIM ≈ 58` at
/// `DIM=384`. The per-doc noise norm is `0.3·√DIM ≈ 5.9` — about
/// 10% of the center magnitude, so docs sit tightly around their
/// planted center direction. If centers were unit-normalized first
/// (`||c|| = 1`), the same per-dim noise of 0.3 would dominate
/// (`||noise|| ≈ 5.9 ≫ ||c|| = 1`), and per-doc normalization would
/// then destroy the cluster signal — IVF + RaBitQ trained on that
/// data can't recover any meaningful cluster structure even at
/// full sweep + maximal rerank. Discovered via the M17 LanceDB
/// head-to-head: a Lance-equivalent corpus generator
/// (`tests/recall.rs::generate_planted_corpus`) keeps cluster
/// signal under cosine; the earlier double-normalize version of
/// this generator did not.
pub fn generate_vector_corpus(
    n_docs: usize,
    n_cent: usize,
    seed: u64,
    normalize_each: bool,
) -> Vec<f32> {
    let mut rng = StdRng::seed_from_u64(seed);
    let dist = StandardNormal;

    // Centers — kept un-normalized; see fn-doc.
    let centers: Vec<Vec<f32>> = (0..n_cent)
        .map(|_| {
            (0..DIM)
                .map(|_| {
                    let s: f64 = dist.sample(&mut rng);
                    (s as f32) * 3.0
                })
                .collect()
        })
        .collect();

    let mut out: Vec<f32> = Vec::with_capacity(n_docs * DIM);
    for i in 0..n_docs {
        let center = &centers[i % n_cent];
        let mut v: Vec<f32> = center
            .iter()
            .map(|&c| {
                let s: f64 = dist.sample(&mut rng);
                c + (s as f32) * 0.3
            })
            .collect();
        if normalize_each {
            normalize(&mut v);
        }
        out.extend_from_slice(&v);
    }
    out
}

/// Build a stand-alone FTS index from a token corpus.
pub fn build_fts_index(docs: &[String]) -> FtsBuilder {
    let mut b = FtsBuilder::new(default_tokenizer());
    b.register_column("title".to_string())
        .expect("register column");
    for (i, text) in docs.iter().enumerate() {
        b.add_doc(0, i as u32, text).expect("add doc");
    }
    b
}

/// Build a stand-alone vector index. `vectors` is flat `n_docs * DIM`.
pub fn build_vector_index(
    vectors: &[f32],
    n_docs: usize,
    n_cent: usize,
    metric: Metric,
) -> VectorBuilder {
    let mut b = VectorBuilder::new();
    b.register_column(VectorConfig {
        name: "v".into(),
        dim: DIM,
        n_cent,
        rot_seed: 7,
        metric,
    })
    .expect("register column");
    for i in 0..n_docs {
        let off = i * DIM;
        b.add(0, &vectors[off..off + DIM])
            .expect("add to vector builder");
    }
    b
}

/// Open a built vector blob as a reader.
pub fn open_vector_reader(blob: Vec<u8>, n_cent: usize, metric: Metric) -> VectorReader {
    let metric_str = match metric {
        Metric::L2Sq => "l2sq",
        Metric::Cosine => "cosine",
        Metric::NegDot => "negdot",
    };
    let json = format!(
        r#"[{{"name":"v","dim":{DIM},"n_cent":{n_cent},"rot_seed":7,"metric":"{metric_str}"}}]"#
    );
    VectorReader::open(Bytes::from(blob), &json).expect("open VectorReader")
}

/// Build a full superfile (FTS + vec) for end-to-end benches.
pub fn build_superfile(docs: &[String], vectors: &[f32], n_cent: usize) -> Vec<u8> {
    use arrow_array::{LargeStringArray, RecordBatch, UInt64Array};
    use arrow_schema::{DataType, Field, Schema};

    let n = docs.len();
    let schema = Arc::new(Schema::new(vec![
        Field::new("doc_id", DataType::UInt64, false),
        Field::new("title", DataType::LargeUtf8, false),
    ]));
    let opts = BuilderOptions::new(
        schema.clone(),
        "doc_id",
        vec![FtsConfig {
            column: "title".into(),
        }],
        vec![SfVectorConfig {
            column: "emb".into(),
            dim: DIM,
            n_cent,
            rot_seed: 7,
            metric: Metric::Cosine,
        }],
        Some(default_tokenizer()),
    );

    let mut b = SuperfileBuilder::new(opts).expect("new SuperfileBuilder");
    let ids = UInt64Array::from((0..n as u64).collect::<Vec<_>>());
    let titles = LargeStringArray::from(docs.iter().map(String::as_str).collect::<Vec<_>>());
    let batch = RecordBatch::try_new(schema, vec![Arc::new(ids), Arc::new(titles)])
        .expect("build RecordBatch");
    b.add_batch(&batch, &[vectors]).expect("add_batch");
    b.finish().expect("finish builder")
}

pub fn open_superfile(bytes: Vec<u8>) -> SuperfileReader {
    SuperfileReader::open(Bytes::from(bytes)).expect("open superfile")
}
