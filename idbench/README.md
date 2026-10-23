# idbench

How much a document id scheme costs a search engine, i.e. what it costs to mint, store and look up an id. The bench compares six schemes:

| scheme | layout | how the bench mints it |
|---|---|---|
| auto-increment | one counter shared by every writer | an atomic `fetch_add` per id, or one block per batch |
| UUIDv4 | 122 random bits | `Uuid::new_v4` with `fast-rng`, or the random bytes for a whole batch at once |
| UUIDv7 | 48-bit ms timestamp, then counter and random bits | `Uuid::now_v7` (the crate's default, one context per process behind a lock), or a `ContextV7` per writer |
| ULID | 48-bit ms timestamp, then 80 random bits | `Ulid::new`, or the crate's monotonic `Generator` per writer |
| Snowflake-64 | Twitter's layout: 41-bit ms timestamp, 10-bit machine id, 12-bit sequence | ferroid's `BasicSnowflakeGenerator`, one machine id per writer |
| Infino-128 | Infino's `_id`: 64-bit ms timestamp, 40-bit random writer id, 24-bit sequence | `IdGenerator::next_id`, or `reserve_range` per batch as an append does |

Each scheme is minted one id at a time and, where the scheme allows it, in batches of 1,024 in its fastest form: a block from the counter, the random bytes for the batch at once, one clock read per batch. Snowflake-64 has no batched row, since its 12-bit sequence caps a writer at 4,096 ids per millisecond, which it already reaches one id at a time. A test checks that every minter yields distinct ids, in increasing order for every scheme that orders them.

In the batch table, auto-increment is the baseline: a writer takes a block of 1,024 numbers from one shared counter with a single atomic add and fills it in, so an id is just the next number. It is as cheap as minting gets, but every writer shares the counter, which across machines means a coordinator. The other rows show how close each scheme gets to it without that coordination.

Everything Infino-specific is the engine's own code, at the commit pinned in `Cargo.toml`. Ids are minted by its `IdGenerator`, every superfile is built by its `SuperfileBuilder`, and hits are resolved to ids by its `SuperfileReader`. The other schemes' ids go through the same builder and reader, so the only thing that differs between the rows of two tables is the id.

## Numbers as of Infino `c5a80fa9`, 23 Oct 2026

Measured on 23 Oct 2026 with Infino at [`c5a80fa9`](https://github.com/infino-ai/infino/commit/c5a80fa9800d5d0004ae0d0ff1493923e3707d66) and this repository at `d06f0a8`, on a Google Cloud `c3-highcpu-8` VM: Intel Xeon Platinum 8481C (Sapphire Rapids) at 2.7 GHz, 8 vCPUs (4 cores with 2 hyperthreads each), 16 GB of memory, Rocky Linux 9.8, Rust 1.95.0, nothing else running. The tables below are copied verbatim from [`results/2026-10-23-gcp-c3-highcpu-8.md`](results/2026-10-23-gcp-c3-highcpu-8.md), which `run.sh` wrote and stamped.

cpus: 8, ids per thread: 5000000, median of 5 runs

### Minting one id at a time

| scheme | 1 writer | 8 writers | ns per id, 1 writer |
|---|---:|---:|---:|
| auto-increment | 119.0 M/s | 41.6 M/s | 8.4 |
| UUIDv4 | 107.2 M/s | 566.1 M/s | 9.3 |
| UUIDv7 (`now_v7`) | 13.5 M/s | 4.2 M/s | 73.9 |
| UUIDv7, a ContextV7 per writer | 14.8 M/s | 95.1 M/s | 67.4 |
| ULID (`Ulid::new`) | 24.4 M/s | 148.4 M/s | 41.1 |
| ULID, monotonic Generator per writer | 26.0 M/s | 161.9 M/s | 38.5 |
| Snowflake-64 | 4.1 M/s | 32.5 M/s | 244.0 |
| Infino-128 | 393.3 M/s | 1.63 B/s | 2.5 |

### Minting in batches of 1024

| scheme | 1 writer | 8 writers | ns per id, 1 writer |
|---|---:|---:|---:|
| auto-increment | 2.82 B/s | 9.74 B/s | 0.35 |
| UUIDv4 | 128.0 M/s | 666.5 M/s | 7.81 |
| UUIDv7, a ContextV7 per writer | 40.7 M/s | 185.9 M/s | 24.59 |
| ULID, monotonic Generator per writer | 132.3 M/s | 452.1 M/s | 7.56 |
| Infino-128 | 2.19 B/s | 8.69 B/s | 0.46 |

### One Infino-128 generator shared by 8 threads

| lock taken | ids per second |
|---|---:|
| for every id | 5.7 M/s |
| once per 1024 ids | 4.23 B/s |

### Bytes per id, one superfile of 1000000 docs from one writer

| scheme | raw | Parquet `_id` column | packed id sidecar | Parquet `_id` pages | encodings |
|---|---:|---:|---:|---:|---|
| auto-increment | 8 | 2.30 | 1.28 | 977 | PLAIN RLE |
| UUIDv4 | 16 | 16.03 | 15.53 | 977 | PLAIN RLE |
| UUIDv7 (`now_v7`) | 16 | 5.74 | 5.62 | 977 | PLAIN RLE |
| UUIDv7, a ContextV7 per writer | 16 | 5.75 | 5.59 | 977 | PLAIN RLE |
| ULID (`Ulid::new`) | 16 | 10.11 | 10.04 | 977 | PLAIN RLE |
| ULID, monotonic Generator per writer | 16 | 1.08 | 1.62 | 977 | PLAIN RLE |
| Snowflake-64 | 8 | 1.10 | 1.28 | 977 | PLAIN RLE |
| Infino-128 | 16 | 1.08 | 1.28 | 977 | PLAIN RLE |

### Resolving k hits to ids, one superfile of 1000000 docs

| scheme | k | from the Parquet `_id` column | from the id sidecar | speedup |
|---|---:|---:|---:|---:|
| UUIDv4 | 10 | 162 µs | 0.8 µs | 207× |
| UUIDv4 | 100 | 289 µs | 4.3 µs | 68× |
| UUIDv4 | 1000 | 1079 µs | 43.8 µs | 25× |
| Infino-128 | 10 | 268 µs | 0.6 µs | 473× |
| Infino-128 | 100 | 1337 µs | 2.0 µs | 665× |
| Infino-128 | 1000 | 7897 µs | 16.1 µs | 490× |

### Finding a row by id, 100 superfiles × 100000 docs

| scheme | concurrent writers | superfiles scanned | lookup time |
|---|---:|---:|---:|
| auto-increment | 1 | 1.0 of 100 | 2.3 ms |
| UUIDv4 | 1 | 100.0 of 100 | 86.9 ms |
| UUIDv7 (`now_v7`) | 1 | 1.0 of 100 | 2.1 ms |
| UUIDv7 (`now_v7`) | 4 | 4.0 of 100 | 7.2 ms |
| UUIDv7, a ContextV7 per writer | 1 | 1.0 of 100 | 2.2 ms |
| UUIDv7, a ContextV7 per writer | 4 | 4.0 of 100 | 8.4 ms |
| ULID (`Ulid::new`) | 1 | 1.1 of 100 | 1.9 ms |
| ULID (`Ulid::new`) | 4 | 4.4 of 100 | 7.5 ms |
| ULID, monotonic Generator per writer | 1 | 1.0 of 100 | 1.9 ms |
| ULID, monotonic Generator per writer | 4 | 4.0 of 100 | 7.6 ms |
| Snowflake-64 | 1 | 1.0 of 100 | 2.0 ms |
| Snowflake-64 | 4 | 4.0 of 100 | 8.1 ms |
| Infino-128 | 1 | 1.0 of 100 | 2.0 ms |
| Infino-128 | 4 | 4.0 of 100 | 7.6 ms |

## Running it

```sh
cd idbench
cargo run --release           # the full run, a few minutes on 8 vCPUs after the build
cargo run --release -- quick  # smaller sizes, under a minute
cargo run --release -- mint   # only the minting tables
```

The toolchain is the repository's, from `rust-toolchain.toml`, i.e. the same one the engine pins. Each run prints its tables as markdown. The absolute numbers depend on the machine, but the ratios between schemes should hold. Writer counts follow the machine's logical CPU count.

To publish a result, commit first and then run:

```sh
./run.sh gcp-c3-highcpu-8
```

It refuses to run with uncommitted changes under `idbench/`. It writes `results/<date>-<machine>.md` with the tables and what produced them: the CPU, memory, OS, `rustc`, the exact Infino commit from `Cargo.lock`, this repository's commit, and the load average when the run started. A starting load well above zero means something else was using the machine, or had just stopped. The tables in this README are copied verbatim from such a file.

To measure a later Infino, change the `rev` of the `infino` dependency in `Cargo.toml`, run `cargo update -p infino`, commit, and run `./run.sh` again.
