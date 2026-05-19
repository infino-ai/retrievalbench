//! Hybrid supertable build throughput across writer-pool sizes.
//!
//! For each `writer_pool` size in {1, 4, num_cpus}, build a fresh
//! supertable from the cached corpus and time the
//! `append + commit` cycle. Throughput in docs/sec via
//! `Throughput::Elements`.
//!
//! After the timed runs the bench prints a one-shot
//! `resident_bytes` + per-doc resident-bytes summary so manual
//! review can spot regressions in the write path's amplification
//! factor.

use crate::hybrid_common::*;
use criterion::{Criterion, Throughput, criterion_group};
use std::hint::black_box;

fn bench_supertable_build(c: &mut Criterion) {
    let corpus = corpus();
    let n = corpus.n;

    let configs: [(&str, usize); 2] = [
        ("4_threads", 4),
        ("num_cpus_threads", num_cpus::get().max(1)),
    ];

    let mut g = c.benchmark_group("supertable_build");
    g.sample_size(10);
    g.throughput(Throughput::Elements(n as u64));

    for (label, writer_threads) in &configs {
        g.bench_function(*label, |b| {
            b.iter_with_large_drop(|| build_supertable(black_box(corpus), *writer_threads));
        });
    }

    g.finish();

    // Memory-budget sanity print: build once at the default
    // writer_threads, report resident bytes per doc. Stderr so
    // criterion's report doesn't drown it.
    let st = build_supertable(corpus, num_cpus::get().max(1));
    let resident = st.options().store.resident_bytes();
    let r = st.reader();
    eprintln!(
        "[hybrid_build] resident_bytes={} n_docs={} n_superfiles={} bytes_per_doc={:.1}",
        resident,
        r.n_docs_total(),
        r.n_superfiles(),
        resident as f64 / (r.n_docs_total() as f64).max(1.0),
    );
}

criterion_group!(benches, bench_supertable_build);
