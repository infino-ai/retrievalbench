// SPDX-License-Identifier: Apache-2.0
//
// How much an id scheme costs a search engine: to mint, to store, and to
// look up. Six schemes, each as a 128-bit value so the storage and lookup
// code is shared:
//
//   auto-increment  one shared counter (an AtomicU64 across threads)
//   UUIDv4          122 random bits
//   UUIDv7          48-bit ms timestamp + counter/random bits
//   ULID            48-bit ms timestamp + 80 random bits
//   Snowflake-64    Twitter layout: 41-bit ms timestamp, 10-bit machine, 12-bit sequence
//   Infino-128      64-bit ms timestamp, 40-bit random worker, 24-bit sequence
//
// Everything Infino-specific is the engine's own code, at the commit pinned
// in Cargo.toml: ids are minted by its IdGenerator (one at a time, and in
// batches the way an append reserves them), every superfile is built by its
// SuperfileBuilder (Parquet body, full-text index and the packed id
// sidecar), and hits are resolved to ids by its SuperfileReader. The other
// schemes' ids go through the same builder and reader, so the only thing
// that differs between rows of a table is the id.
//
// Run:  cargo run --release            (a few minutes on 8 cores, after the build)
//       cargo run --release -- quick   (smaller sizes, for a smoke test)
//       cargo run --release -- mint    (only the minting tables)

use std::{
    hint::black_box,
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Instant,
};

use arrow_array::{Decimal128Array, LargeStringArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use bytes::Bytes;
use ferroid::{
    generator::BasicSnowflakeGenerator,
    id::SnowflakeTwitterId,
    time::{MonotonicClock, TWITTER_EPOCH},
};
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scheme {
    AutoInc,
    UuidV4,
    UuidV7,
    Ulid,
    Snowflake64,
    Infino128,
}

const SCHEMES: [Scheme; 6] = [
    Scheme::AutoInc,
    Scheme::UuidV4,
    Scheme::UuidV7,
    Scheme::Ulid,
    Scheme::Snowflake64,
    Scheme::Infino128,
];

impl Scheme {
    fn name(self) -> &'static str {
        match self {
            Scheme::AutoInc => "auto-increment",
            Scheme::UuidV4 => "UUIDv4",
            Scheme::UuidV7 => "UUIDv7",
            Scheme::Ulid => "ULID",
            Scheme::Snowflake64 => "Snowflake-64",
            Scheme::Infino128 => "Infino-128",
        }
    }
    fn raw_bytes(self) -> usize {
        match self {
            Scheme::AutoInc | Scheme::Snowflake64 => 8,
            _ => 16,
        }
    }
}

/// One writer's id source. `shared` is the cross-thread state a scheme
/// needs (only auto-increment has any).
enum Minter {
    AutoInc(Arc<AtomicU64>),
    UuidV4,
    UuidV7,
    Ulid,
    Snowflake64(BasicSnowflakeGenerator<SnowflakeTwitterId, MonotonicClock>),
    Infino128(IdGenerator),
}

impl Minter {
    /// `writer` is this writer's index; Snowflake-64 needs it as its
    /// machine id, because a 10-bit machine field is too small to pick at
    /// random (two of 32 writers collide with 38% probability), so someone
    /// has to hand the numbers out. Infino-128 ignores it and draws 40
    /// random bits instead, which is why its writers need no coordination.
    fn new(s: Scheme, counter: &Arc<AtomicU64>, writer: u64) -> Self {
        match s {
            Scheme::AutoInc => Minter::AutoInc(counter.clone()),
            Scheme::UuidV4 => Minter::UuidV4,
            Scheme::UuidV7 => Minter::UuidV7,
            Scheme::Ulid => Minter::Ulid,
            Scheme::Snowflake64 => Minter::Snowflake64(BasicSnowflakeGenerator::new(
                writer & 0x3ff,
                MonotonicClock::<1>::with_epoch(TWITTER_EPOCH),
            )),
            Scheme::Infino128 => Minter::Infino128(IdGenerator::new()),
        }
    }

    #[inline]
    fn next(&self) -> u128 {
        match self {
            Minter::AutoInc(c) => c.fetch_add(1, Ordering::Relaxed) as u128,
            Minter::UuidV4 => uuid::Uuid::new_v4().as_u128(),
            Minter::UuidV7 => uuid::Uuid::now_v7().as_u128(),
            Minter::Ulid => ulid::Ulid::new().0,
            Minter::Snowflake64(g) => g.next_id(|_| std::hint::spin_loop()).to_raw() as u128,
            Minter::Infino128(g) => g.next_id() as u128,
        }
    }
}

// ---------------------------------------------------------------- generation

/// Ids per second, minted by `threads` writers, each minting `per_thread`.
fn mint_rate(s: Scheme, threads: usize, per_thread: usize) -> f64 {
    let counter = Arc::new(AtomicU64::new(0));
    let barrier = Arc::new(Barrier::new(threads + 1));
    let handles: Vec<_> = (0..threads)
        .map(|w| {
            let counter = counter.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                let m = Minter::new(s, &counter, w as u64);
                m.next(); // start the clock's ticker before timing
                barrier.wait();
                let mut acc = 0u128;
                for _ in 0..per_thread {
                    acc ^= m.next();
                }
                black_box(acc);
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

/// Ids for `n` rows the way the engine's append reserves them: one call to
/// `reserve_range`, which returns contiguous spans, expanded into `buf`.
/// (`try_into` because older engine versions take the count as `u32`.)
fn reserve_into(g: &IdGenerator, n: usize, buf: &mut Vec<i128>) {
    let spans = g.reserve_range(n.try_into().unwrap());
    expand(&spans, buf);
}

fn expand(spans: &[(i128, i128)], buf: &mut Vec<i128>) {
    buf.clear();
    for &(first, last) in spans {
        let len = (last - first) as usize + 1;
        buf.extend((0..len).map(|i| first + i as i128));
    }
}

/// Infino's append path: each writer has its own generator and reserves
/// ids for `batch` rows at a time.
fn mint_rate_infino_batched(threads: usize, per_thread: usize, batch: usize) -> f64 {
    let barrier = Arc::new(Barrier::new(threads + 1));
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let barrier = barrier.clone();
            thread::spawn(move || {
                let g = IdGenerator::new();
                g.next_id(); // start the clock's ticker before timing
                let mut buf: Vec<i128> = Vec::with_capacity(batch);
                barrier.wait();
                let mut left = per_thread;
                while left > 0 {
                    let n = left.min(batch);
                    reserve_into(&g, n, &mut buf);
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

/// Infino's shared-handle case: writers share one generator behind a mutex
/// and take the lock once per batch of `batch` rows, as an append does: the
/// ids are reserved under the lock and expanded after it is released. With
/// `batch == 1` the lock is taken for every id.
fn mint_rate_infino_shared(threads: usize, per_thread: usize, batch: usize) -> f64 {
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
                    let spans = g.lock().unwrap().reserve_range(n.try_into().unwrap());
                    expand(&spans, &mut buf);
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

/// `n` ids from one writer, in minting order: what one superfile holds.
fn mint_run(s: Scheme, n: usize) -> Vec<u128> {
    let counter = Arc::new(AtomicU64::new(0));
    let m = Minter::new(s, &counter, 1);
    (0..n).map(|_| m.next()).collect()
}

// ------------------------------------------------------------- superfiles

/// Decimal128(38, 0) holds values below 10^38, about 2^126.2. Time-ordered
/// ids and counters are far below that; a UUIDv4's top two bits are
/// random, so they are cleared, leaving 120 random bits instead of 122.
const ID_MASK: u128 = u128::MAX >> 2;

/// One superfile, built by the engine: `ids` as its `_id` column and a
/// short full-text column, so the file has the shape a real one has.
fn build_superfile(ids: &[u128]) -> SuperfileReader {
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
        let id_arr = Decimal128Array::from(
            chunk
                .iter()
                .map(|&v| (v & ID_MASK) as i128)
                .collect::<Vec<_>>(),
        )
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
    println!("cpus: {threads}, ids per thread: {per_thread}, median of {reps} runs\n");

    // 1. Minting.
    println!("## Minting\n");
    println!("| scheme | 1 writer | {threads} writers | ns per id, 1 writer |");
    println!("|---|---:|---:|---:|");
    for s in SCHEMES {
        let one = median((0..reps).map(|_| mint_rate(s, 1, per_thread)).collect());
        let many = median(
            (0..reps)
                .map(|_| mint_rate(s, threads, per_thread))
                .collect(),
        );
        println!(
            "| {} | {} | {} | {:.1} |",
            s.name(),
            fmt_rate(one),
            fmt_rate(many),
            1e9 / one
        );
    }
    let one = median(
        (0..reps)
            .map(|_| mint_rate_infino_batched(1, per_thread, 1024))
            .collect(),
    );
    let many = median(
        (0..reps)
            .map(|_| mint_rate_infino_batched(threads, per_thread, 1024))
            .collect(),
    );
    println!(
        "| Infino-128, batches of 1024 ids (the append path) | {} | {} | {:.2} |",
        fmt_rate(one),
        fmt_rate(many),
        1e9 / one
    );
    for batch in [1usize, 1024] {
        let many = median(
            (0..reps)
                .map(|_| mint_rate_infino_shared(threads, per_thread, batch))
                .collect(),
        );
        println!(
            "| Infino-128, one shared generator, lock per {} | | {} | |",
            if batch == 1 {
                "id".to_string()
            } else {
                format!("{batch} ids")
            },
            fmt_rate(many)
        );
    }

    if std::env::args().any(|a| a == "mint") {
        return;
    }

    // 2. Storage of one superfile's ids.
    println!("\n## Bytes per id, one superfile of {file_docs} docs from one writer\n");
    println!(
        "| scheme | raw | Parquet `_id` column | packed id sidecar | Parquet `_id` pages | encodings |"
    );
    println!("|---|---:|---:|---:|---:|---|");
    let mut runs = Vec::new();
    for s in SCHEMES {
        let ids = mint_run(s, file_docs);
        let sf = build_superfile(&ids);
        let size = sf.size_breakdown().unwrap().expect("size breakdown");
        let col = size
            .columns
            .iter()
            .find(|(n, _)| n == "_id")
            .map(|&(_, b)| b)
            .unwrap();
        let (pages, encodings) = id_pages(&sf);
        println!(
            "| {} | {} | {:.2} | {:.2} | {pages} | {encodings} |",
            s.name(),
            s.raw_bytes(),
            col as f64 / ids.len() as f64,
            size.id_sidecar_bytes as f64 / ids.len() as f64
        );
        runs.push((s, ids, sf));
    }

    // 3. Resolving hits to ids: what every search does for its top-k.
    println!("\n## Resolving k hits to ids, one superfile of {file_docs} docs\n");
    println!("| scheme | k | from the Parquet `_id` column | from the id sidecar | speedup |");
    println!("|---|---:|---:|---:|---:|");
    let mut rng = rand::rng();
    for (s, ids, sf) in runs
        .iter()
        .filter(|(s, _, _)| matches!(s, Scheme::UuidV4 | Scheme::Infino128))
    {
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
                let want: Vec<i128> = q
                    .iter()
                    .map(|&d| (ids[d as usize] & ID_MASK) as i128)
                    .collect();
                assert_eq!(parquet_ids(sf, Some(q)), want);
                assert_eq!(sidecar_ids(sf, q), want);
            }
            let pq_us = median(
                (0..reps)
                    .map(|_| {
                        let t = Instant::now();
                        for q in &queries {
                            black_box(parquet_ids(sf, Some(q)));
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
                            black_box(sidecar_ids(sf, q));
                        }
                        t.elapsed().as_secs_f64() * 1e6 / queries.len() as f64
                    })
                    .collect(),
            );
            println!(
                "| {} | {k} | {pq_us:.0} µs | {sc_us:.1} µs | {:.0}× |",
                s.name(),
                pq_us / sc_us
            );
        }
    }
    drop(runs);

    // 4. Finding rows by id across a table: what a delete or an update by
    //    `_id` does first. The manifest keeps each superfile's id min and
    //    max; a superfile whose range cannot hold the id is skipped, and
    //    every other one has its `_id` column scanned.
    println!("\n## Finding a row by id, {n_files} superfiles × {docs_per_file} docs\n");
    println!("| scheme | concurrent writers | superfiles scanned | lookup time |");
    println!("|---|---:|---:|---:|");
    for s in SCHEMES {
        for writers in [1usize, 4] {
            if writers > 1 && matches!(s, Scheme::AutoInc | Scheme::UuidV4) {
                continue;
            }
            // `writers` writers ingest at once, each into its own
            // superfiles, so their time ranges overlap.
            let counter = Arc::new(AtomicU64::new(0));
            let minters: Vec<Minter> = (0..writers)
                .map(|w| Minter::new(s, &counter, w as u64))
                .collect();
            let mut files: Vec<Vec<u128>> = vec![Vec::with_capacity(docs_per_file); n_files];
            for f in 0..n_files / writers {
                for _ in 0..docs_per_file {
                    for (w, m) in minters.iter().enumerate() {
                        files[f * writers + w].push(m.next());
                    }
                }
            }
            let ranges: Vec<(i128, i128)> = files
                .iter()
                .map(|f| {
                    let v = f.iter().map(|&x| (x & ID_MASK) as i128);
                    (v.clone().min().unwrap(), v.max().unwrap())
                })
                .collect();
            let sfs: Vec<SuperfileReader> = files.iter().map(|f| build_superfile(f)).collect();
            let targets: Vec<i128> = (0..20)
                .map(|_| {
                    (files[rng.random_range(0..n_files)][rng.random_range(0..docs_per_file)]
                        & ID_MASK) as i128
                })
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
                "| {} | {writers} | {:.1} of {n_files} | {ms:.1} ms |",
                s.name(),
                scanned as f64 / targets.len() as f64
            );
        }
    }
}
