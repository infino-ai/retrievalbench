//! Hybrid supertable append-without-commit latency.
//!
//! Measures the cost of a single `append()` call (Arrow validation
//! + vector split + buffer accounting), without the commit-time
//! rayon-shard work. Fresh supertable + writer per iter so the
//! buffer size doesn't compound across iterations.

use crate::common::*;
use criterion::{Criterion, criterion_group};
use infino::supertable::Supertable;
use std::hint::black_box;

fn bench_supertable_append(c: &mut Criterion) {
    let corpus = corpus();
    let batches = batches();
    let mut g = c.benchmark_group("supertable_append");
    g.sample_size(10);

    g.bench_function("single_chunk_append_no_commit", |b| {
        b.iter_with_setup(
            || {
                let st = Supertable::create(supertable_options(corpus.n_cent, 1))
                    .expect("create supertable");
                let w = st.writer().expect("writer");
                (st, w)
            },
            |(_st, mut w)| {
                w.append(black_box(&batches[0])).expect("append");
                // Discard without commit — we're timing append only.
                drop(w);
            },
        );
    });

    g.finish();
}

criterion_group!(benches, bench_supertable_append);
