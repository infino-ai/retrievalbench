// SPDX-License-Identifier: Apache-2.0
//
// What Infino's document id costs: to mint, to store, to resolve from a
// search hit, and to find a row by. Infino's `_id` is a 128-bit
// Snowflake-style id: a 64-bit millisecond timestamp, a 40-bit random
// writer id and a 24-bit sequence number.
//
// Everything measured is the engine's own code, at the commit pinned in
// Cargo.toml: ids are minted by its IdGenerator (one at a time, and in
// batches the way an append reserves them), every superfile is built by its
// SuperfileBuilder (Parquet body, full-text index and the packed id
// sidecar), and hits are resolved to ids by its SuperfileReader.
//
// Run:  cargo run --release            (a few minutes on 8 cores, after the build)
//       cargo run --release -- quick   (smaller sizes, for a smoke test)
//       cargo run --release -- mint    (only the minting tables)

use std::{
    hint::black_box,
    sync::{Arc, Barrier, Mutex},
    thread,
    time::Instant,
};

use arrow_array::{Decimal128Array, LargeStringArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use bytes::Bytes;
use infino::{
    superfile::{
        SuperfileReader,
        builder::{BuilderOptions, FtsConfig, SuperfileBuilder},
    },
    supertable::utils::idgen::IdGenerator,
};
use parquet::{
    arrow::{
        ProjectionMask,
        arrow_reader::{
            ArrowReaderOptions, ParquetRecordBatchReaderBuilder, RowSelection, RowSelector,
        },
    },
    file::metadata::PageIndexPolicy,
};
use rand::Rng;

/// The batch size an append reserves ids in, for the batched tables.
const BATCH: usize = 1024;

// ---------------------------------------------------------------- minting

/// Ids per second, minted by `threads` writers, each with its own
/// generator and each minting `per_thread` ids. With `batch == 1` every id
/// is a `next_id` call; otherwise a writer reserves `batch` ids at a time
/// with `reserve_range` and fills them in, as an append does.
fn mint_rate(threads: usize, per_thread: usize, batch: usize) -> f64 {
    let barrier = Arc::new(Barrier::new(threads + 1));
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let barrier = barrier.clone();
            thread::spawn(move || {
                let g = IdGenerator::new();
                let mut buf: Vec<i128> = Vec::with_capacity(batch);
                g.next_id(); // start the clock's ticker before timing
                barrier.wait();
                if batch == 1 {
                    let mut acc = 0i128;
                    for _ in 0..per_thread {
                        acc ^= g.next_id();
                    }
                    black_box(acc);
                } else {
                    let mut left = per_thread;
                    while left > 0 {
                        let n = left.min(batch);
                        fill(&g.reserve_range(n), &mut buf);
                        black_box(&buf);
                        left -= n;
                    }
                }
                barrier.wait();
            })
        })
        .collect();
    barrier.wait();
    let t = Instant::now();
    barrier.wait();
    let secs = t.elapsed().as_secs_f64();
    handles.into_iter().for_each(|h| h.join().unwrap());
    (threads * per_thread) as f64 / secs
}

/// Expand `reserve_range`'s spans into the ids they hold, as an append
/// fills in its batch.
fn fill(spans: &[(i128, i128)], buf: &mut Vec<i128>) {
    buf.clear();
    for &(first, last) in spans {
        buf.extend(first..=last);
    }
}

/// The shared-handle case: writers share one generator behind a mutex and
/// take the lock once per batch of `batch` rows, as an append does: the
/// ids are reserved under the lock and filled in after it is released.
/// With `batch == 1` the lock is taken for every id.
fn mint_rate_shared(threads: usize, per_thread: usize, batch: usize) -> f64 {
    let g = Arc::new(Mutex::new(IdGenerator::new()));
    let barrier = Arc::new(Barrier::new(threads + 1));
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let g = g.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                let mut buf: Vec<i128> = Vec::with_capacity(batch);
                barrier.wait();
                let mut left = per_thread;
                while left > 0 {
                    let n = left.min(batch);
                    let spans = g.lock().unwrap().reserve_range(n);
                    fill(&spans, &mut buf);
                    black_box(&buf);
                    left -= n;
                }
                barrier.wait();
            })
        })
        .collect();
    barrier.wait();
    let t = Instant::now();
    barrier.wait();
    let secs = t.elapsed().as_secs_f64();
    handles.into_iter().for_each(|h| h.join().unwrap());
    (threads * per_thread) as f64 / secs
}

// ------------------------------------------------------------- superfiles

/// One superfile, built by the engine: `ids` as its `_id` column and a
/// short full-text column, so the file has the shape a real one has.
fn build_superfile(ids: &[i128]) -> SuperfileReader {
    let schema = Arc::new(Schema::new(vec![
        Field::new("_id", DataType::Decimal128(38, 0), false),
        Field::new("body", DataType::LargeUtf8, false),
    ]));
    let opts = BuilderOptions::new(schema.clone(), "_id", vec![FtsConfig::new("body")], vec![]);
    let mut builder = SuperfileBuilder::new(opts).expect("builder");
    const WORDS: [&str; 8] = [
        "refund", "invoice", "outage", "order", "login", "delay", "billing", "error",
    ];
    for chunk in ids.chunks(65_536) {
        let id_arr = Decimal128Array::from(chunk.to_vec())
            .with_precision_and_scale(38, 0)
            .unwrap();
        let body = LargeStringArray::from_iter_values(chunk.iter().map(|&v| {
            format!(
                "{} {} ticket",
                WORDS[(v % 8) as usize],
                WORDS[((v >> 3) % 8) as usize]
            )
        }));
        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(id_arr), Arc::new(body)]).unwrap();
        builder.add_batch(&batch, &[]).expect("add_batch");
    }
    SuperfileReader::open(Bytes::from(builder.finish().expect("finish"))).expect("open")
}

/// The `_id` column of a superfile, read straight from its Parquet body:
/// the rows at `docs` (ascending), or every row when `docs` is `None`.
/// With a row selection and the page index, only pages holding a
/// requested row are decoded.
fn parquet_ids(sf: &SuperfileReader, docs: Option<&[u32]>) -> Vec<i128> {
    let bytes = sf.parquet_bytes().expect("in-memory superfile").clone();
    let mut b = ParquetRecordBatchReaderBuilder::try_new_with_options(
        bytes,
        ArrowReaderOptions::new().with_page_index_policy(PageIndexPolicy::Optional),
    )
    .unwrap();
    let mask = ProjectionMask::roots(b.parquet_schema(), [0]);
    b = b.with_projection(mask);
    if let Some(docs) = docs {
        let n_rows = sf.n_docs() as usize;
        let mut sel = Vec::with_capacity(docs.len() * 2 + 1);
        let mut at = 0usize;
        for &d in docs {
            let d = d as usize;
            if d > at {
                sel.push(RowSelector::skip(d - at));
            }
            sel.push(RowSelector::select(1));
            at = d + 1;
        }
        if n_rows > at {
            sel.push(RowSelector::skip(n_rows - at));
        }
        b = b.with_row_selection(RowSelection::from(sel));
    }
    let mut out = Vec::new();
    for batch in b.build().unwrap() {
        let batch = batch.unwrap();
        let a = batch
            .column(0)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        out.extend(a.values().iter().copied());
    }
    out
}

/// How the engine laid out the `_id` column: data pages across all row
/// groups, and the encodings it chose.
fn id_pages(sf: &SuperfileReader) -> (usize, String) {
    let bytes = sf.parquet_bytes().expect("in-memory superfile").clone();
    let b = ParquetRecordBatchReaderBuilder::try_new_with_options(
        bytes,
        ArrowReaderOptions::new().with_page_index_policy(PageIndexPolicy::Optional),
    )
    .unwrap();
    let md = b.metadata();
    let pages = md
        .offset_index()
        .map(|oi| oi.iter().map(|rg| rg[0].page_locations().len()).sum())
        .unwrap_or(0);
    let mut enc: Vec<String> = md
        .row_groups()
        .iter()
        .flat_map(|rg| {
            rg.column(0)
                .encodings()
                .map(|e| format!("{e:?}"))
                .collect::<Vec<_>>()
        })
        .collect();
    enc.sort();
    enc.dedup();
    (pages, enc.join(" "))
}

/// The `_id`s of the hits at `docs`, through the engine's own resolve:
/// with `_id` as the only projection it reads the packed sidecar and never
/// touches a Parquet page.
fn sidecar_ids(sf: &SuperfileReader, docs: &[u32]) -> Vec<i128> {
    let batch = sf.take_by_local_doc_ids(docs, &["_id"]).expect("take");
    let a = batch
        .column(0)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .unwrap();
    a.values().to_vec()
}

// -------------------------------------------------------------------- report

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn fmt_rate(r: f64) -> String {
    if r >= 1e9 {
        format!("{:.2} B/s", r / 1e9)
    } else {
        format!("{:.1} M/s", r / 1e6)
    }
}

fn main() {
    let quick = std::env::args().any(|a| a == "quick");
    let threads = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let per_thread = if quick { 1_000_000 } else { 5_000_000 };
    let file_docs = if quick { 100_000 } else { 1_000_000 };
    let n_files = if quick { 20 } else { 100 };
    let docs_per_file = if quick { 10_000 } else { 100_000 };
    let reps = 5;

    println!("# idbench\n");
    println!("cpus: {threads}, ids per writer: {per_thread}, median of {reps} runs\n");

    // 1. Minting, each writer with its own generator.
    println!("## Minting\n");
    println!("| how | 1 writer | {threads} writers | ns per id, 1 writer |");
    println!("|---|---:|---:|---:|");
    for batch in [1usize, BATCH] {
        let one = median((0..reps).map(|_| mint_rate(1, per_thread, batch)).collect());
        let many = median(
            (0..reps)
                .map(|_| mint_rate(threads, per_thread, batch))
                .collect(),
        );
        println!(
            "| {} | {} | {} | {:.2} |",
            if batch == 1 {
                "one id at a time (`next_id`)".to_string()
            } else {
                format!("in batches of {batch}, as an append reserves them (`reserve_range`)")
            },
            fmt_rate(one),
            fmt_rate(many),
            1e9 / one
        );
    }

    // 2. One generator shared by every thread, as appends through one
    //    supertable handle share it.
    println!("\n## One generator shared by {threads} threads\n");
    println!("| lock taken | ids per second |");
    println!("|---|---:|");
    for batch in [1usize, BATCH] {
        let many = median(
            (0..reps)
                .map(|_| mint_rate_shared(threads, per_thread, batch))
                .collect(),
        );
        println!(
            "| {} | {} |",
            if batch == 1 {
                "for every id".to_string()
            } else {
                format!("once per {batch} ids")
            },
            fmt_rate(many)
        );
    }

    if std::env::args().any(|a| a == "mint") {
        return;
    }

    // 3. Storage of one superfile's ids.
    let g = IdGenerator::new();
    let ids: Vec<i128> = (0..file_docs).map(|_| g.next_id()).collect();
    let sf = build_superfile(&ids);
    let size = sf.size_breakdown().unwrap().expect("size breakdown");
    let col = size
        .columns
        .iter()
        .find(|(n, _)| n == "_id")
        .map(|&(_, b)| b)
        .unwrap();
    let (pages, encodings) = id_pages(&sf);
    println!("\n## Bytes per id, one superfile of {file_docs} docs from one writer\n");
    println!(
        "| raw | Parquet `_id` column | packed id sidecar | Parquet `_id` pages | encodings |"
    );
    println!("|---:|---:|---:|---:|---|");
    println!(
        "| 16 | {:.2} | {:.2} | {pages} | {encodings} |",
        col as f64 / ids.len() as f64,
        size.id_sidecar_bytes as f64 / ids.len() as f64
    );

    // 4. Resolving hits to ids: what every search does for its top-k.
    println!("\n## Resolving k hits to ids, one superfile of {file_docs} docs\n");
    println!("| k | from the Parquet `_id` column | from the id sidecar | speedup |");
    println!("|---:|---:|---:|---:|");
    let mut rng = rand::rng();
    for k in [10usize, 100, 1000] {
        let queries: Vec<Vec<u32>> = (0..50)
            .map(|_| {
                let mut d: Vec<u32> = (0..k)
                    .map(|_| rng.random_range(0..ids.len() as u32))
                    .collect();
                d.sort_unstable();
                d.dedup();
                d
            })
            .collect();
        for q in &queries[..3] {
            let want: Vec<i128> = q.iter().map(|&d| ids[d as usize]).collect();
            assert_eq!(parquet_ids(&sf, Some(q)), want);
            assert_eq!(sidecar_ids(&sf, q), want);
        }
        let pq_us = median(
            (0..reps)
                .map(|_| {
                    let t = Instant::now();
                    for q in &queries {
                        black_box(parquet_ids(&sf, Some(q)));
                    }
                    t.elapsed().as_secs_f64() * 1e6 / queries.len() as f64
                })
                .collect(),
        );
        let sc_us = median(
            (0..reps)
                .map(|_| {
                    let t = Instant::now();
                    for q in &queries {
                        black_box(sidecar_ids(&sf, q));
                    }
                    t.elapsed().as_secs_f64() * 1e6 / queries.len() as f64
                })
                .collect(),
        );
        println!(
            "| {k} | {pq_us:.0} µs | {sc_us:.1} µs | {:.0}× |",
            pq_us / sc_us
        );
    }
    drop((ids, sf));

    // 5. Finding rows by id across a table: what a delete or an update by
    //    `_id` does first. The manifest keeps each superfile's id min and
    //    max; a superfile whose range cannot hold the id is skipped, and
    //    every other one has its `_id` column scanned.
    println!("\n## Finding a row by id, {n_files} superfiles × {docs_per_file} docs\n");
    println!("| concurrent writers | superfiles scanned | lookup time |");
    println!("|---:|---:|---:|");
    for writers in [1usize, 4] {
        // `writers` writers ingest at once, each into its own superfiles,
        // so their time ranges overlap.
        let gens: Vec<IdGenerator> = (0..writers).map(|_| IdGenerator::new()).collect();
        let mut files: Vec<Vec<i128>> = vec![Vec::with_capacity(docs_per_file); n_files];
        for f in 0..n_files / writers {
            for _ in 0..docs_per_file {
                for (w, g) in gens.iter().enumerate() {
                    files[f * writers + w].push(g.next_id());
                }
            }
        }
        let ranges: Vec<(i128, i128)> = files
            .iter()
            .map(|f| (*f.iter().min().unwrap(), *f.iter().max().unwrap()))
            .collect();
        let sfs: Vec<SuperfileReader> = files.iter().map(|f| build_superfile(f)).collect();
        let targets: Vec<i128> = (0..20)
            .map(|_| files[rng.random_range(0..n_files)][rng.random_range(0..docs_per_file)])
            .collect();
        let mut scanned = 0usize;
        let t = Instant::now();
        for &id in &targets {
            let mut found = false;
            for (f, &(lo, hi)) in ranges.iter().enumerate() {
                if id < lo || id > hi {
                    continue;
                }
                scanned += 1;
                found |= parquet_ids(&sfs[f], None).contains(&id);
            }
            assert!(found);
        }
        let ms = t.elapsed().as_secs_f64() * 1e3 / targets.len() as f64;
        println!(
            "| {writers} | {:.1} of {n_files} | {ms:.1} ms |",
            scanned as f64 / targets.len() as f64
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// Both ways the bench mints ids yield distinct ids in increasing
    /// order, so no row of a table is fast because it cut a corner.
    #[test]
    fn minting_yields_distinct_increasing_ids_one_at_a_time_and_in_batches() {
        let g = IdGenerator::new();
        let one: Vec<i128> = (0..20_000).map(|_| g.next_id()).collect();
        let mut batched = Vec::new();
        let mut buf = Vec::new();
        for _ in 0..20 {
            fill(&g.reserve_range(BATCH), &mut buf);
            assert_eq!(buf.len(), BATCH);
            batched.extend_from_slice(&buf);
        }
        for ids in [&one, &batched] {
            assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
            assert!(ids.windows(2).all(|w| w[0] < w[1]));
        }
        assert!(one.last() < batched.first());
    }
}
