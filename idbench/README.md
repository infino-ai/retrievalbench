# idbench

What Infino's document id costs a search engine: to mint, to store, to resolve from a search hit, and to find a row by.

Every row in an Infino table has a system-managed `_id`, a 128-bit Snowflake-style id:

| bits | field |
|---:|---|
| 64 | millisecond timestamp |
| 40 | writer id, drawn at random when a writer starts |
| 24 | sequence number within the millisecond |

Because the writer id is random, writers mint ids without coordinating with each other. Because the timestamp leads, ids sort by the millisecond they were minted in, and they are stored as a plain `Decimal128(38, 0)` Parquet column.

Everything measured is the engine's own code, at the commit pinned in `Cargo.toml`: ids are minted by its `IdGenerator`, superfiles are built by its `SuperfileBuilder`, and hits are resolved by its `SuperfileReader`.

## What it measures

- **Minting:** ids per second with 1 writer and with one writer per logical CPU, each with its own generator. Each rep runs for a fixed second, and the run reports how many threads the process has meanwhile: the writers, the main thread, and the one clock ticker that every generator in a process shares. One id at a time through `next_id`, and in batches of 1,024 through `reserve_range`, which is how an append mints: one clock read reserves a run of ids, and the append fills them in.
- **One generator shared by every thread:** appends through one supertable handle share its generator behind a lock. The lock is taken for every id, and once per batch of 1,024.
- **Bytes per id:** one superfile of 1M ids from one writer. The Parquet `_id` column and the packed id sidecar, against 16 raw bytes.
- **Resolving k hits to ids:** turning the row numbers of a search's top k hits into `_id`s, from the Parquet `_id` column and from the id sidecar, for k = 10, 100 and 1,000. The Parquet footer and page index are decoded once per superfile and reused, as a reader that keeps files open does.
- **Finding a row by id:** what a delete or an update does first. Skip every superfile whose id range cannot hold the id, and scan the `_id` column of the rest. A table of 100 superfiles of 100,000 rows, written by 1 and by 4 concurrent writers.

A test checks that both ways of minting yield distinct ids in increasing order.

## Results

Measured on 5 Nov 2026 with Infino at [`c5a80fa9`](https://github.com/infino-ai/infino/commit/c5a80fa9800d5d0004ae0d0ff1493923e3707d66) and this repository at `78daab8`, on a dedicated Google Cloud `c3-highcpu-8` VM: Intel Xeon Platinum 8481C (Sapphire Rapids) at 2.7 GHz, 8 vCPUs, 16 GB of memory, Rocky Linux 9.8, Rust 1.95.0. The tables are copied verbatim from [`results/2026-11-05-gcp-c3-highcpu-8.md`](results/2026-11-05-gcp-c3-highcpu-8.md), which `run.sh` wrote and stamped.

cpus: 8, each minting rep runs for 1000 ms, median of 5 reps

### Minting

| how | 1 writer | 8 writers | ns per id, 1 writer |
|---|---:|---:|---:|
| one id at a time (`next_id`) | 1.15 B/s | 3.82 B/s | 0.87 |
| in batches of 1024, as an append reserves them (`reserve_range`) | 1.43 B/s | 4.53 B/s | 0.70 |

Threads in the process during a 8-writer rep: 10 (8 writers, the main thread, and the shared clock's ticker, which wakes once a millisecond).

### One generator shared by 8 threads

| lock taken | ids per second |
|---|---:|
| for every id | 4.7 M/s |
| once per 1024 ids | 3.75 B/s |

### Bytes per id, one superfile of 1000000 docs from one writer

| raw | Parquet `_id` column | packed id sidecar | Parquet `_id` pages | encodings |
|---:|---:|---:|---:|---|
| 16 | 1.08 | 1.29 | 977 | PLAIN RLE |

### Resolving k hits to ids, one superfile of 1000000 docs

| k | from the Parquet `_id` column | from the id sidecar | speedup |
|---:|---:|---:|---:|
| 10 | 214 µs | 0.5 µs | 400× |
| 100 | 1279 µs | 2.1 µs | 599× |
| 1000 | 7901 µs | 16.2 µs | 489× |

### Finding a row by id, 100 superfiles × 100000 docs

| concurrent writers | superfiles scanned | lookup time |
|---:|---:|---:|
| 1 | 1.0 of 100 | 1.9 ms |
| 4 | 4.0 of 100 | 7.5 ms |

## Running it

```sh
cd idbench
cargo run --release           # the full run, a few minutes on 8 vCPUs after the build
cargo run --release -- quick  # smaller sizes, under a minute
cargo run --release -- mint   # only the minting tables
```

The toolchain is the repository's, from `rust-toolchain.toml`, the same one the engine pins. Each run prints its tables as markdown. The absolute numbers depend on the machine, and writer counts follow its logical CPU count.

To publish a result, commit first and then run:

```sh
./run.sh gcp-c3-highcpu-8
```

It refuses to run with uncommitted changes under `idbench/`. It writes `results/<date>-<machine>.md` with the tables and what produced them: the CPU, memory, OS, `rustc`, the exact Infino commit from `Cargo.lock`, this repository's commit, and the load average when the run started. A starting load well above zero means something else was using the machine, or had just stopped. The tables in this README are copied verbatim from such a file.

To measure a later Infino, change the `rev` of the `infino` dependency in `Cargo.toml`, run `cargo update -p infino`, commit, and run `./run.sh` again.
