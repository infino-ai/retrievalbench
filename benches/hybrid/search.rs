//! Hybrid supertable query latency across BM25 (term + prefix) and
//! vector query types. Builds the supertable once, times each query
//! shape against the same pre-pinned reader.

use crate::common::*;
use criterion::{Criterion, criterion_group};
use infino::superfile::fts::reader::BoolMode;
use infino::superfile::vector::distance::normalize;
use infino::supertable::query::vector::VectorSearchOptions;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand_distr::{Distribution, StandardNormal};
use retrievalbench::corpus;
use std::hint::black_box;

fn bench_supertable_query(c: &mut Criterion) {
    let st = prebuilt_supertable();
    let r = st.reader();

    // Deterministic unit-norm query vector for the vector search shape.
    let mut rng = StdRng::seed_from_u64(101);
    let dist = StandardNormal;
    let mut q: Vec<f32> = (0..corpus::DIM)
        .map(|_| {
            let s: f64 = dist.sample(&mut rng);
            (s as f32) * 3.0
        })
        .collect();
    normalize(&mut q);

    let mut g = c.benchmark_group("supertable_query");
    g.sample_size(10);

    g.bench_function("bm25_or_top10", |b| {
        b.iter(|| {
            let hits = r
                .bm25_search(
                    black_box("title"),
                    black_box("term00001"),
                    TOP_K,
                    BoolMode::Or,
                )
                .expect("bm25");
            black_box(hits)
        });
    });

    g.bench_function("bm25_prefix_top10", |b| {
        b.iter(|| {
            let hits = r
                .bm25_search_prefix(black_box("title"), black_box("term000"), TOP_K)
                .expect("bm25_prefix");
            black_box(hits)
        });
    });

    g.bench_function("vector_default_top10", |b| {
        b.iter(|| {
            let hits = r
                .vector_search(
                    black_box("emb"),
                    black_box(&q),
                    TOP_K,
                    VectorSearchOptions::new(),
                )
                .expect("vector");
            black_box(hits)
        });
    });

    g.finish();
}

criterion_group!(benches, bench_supertable_query);
