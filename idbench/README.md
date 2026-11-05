# idbench

What Infino's document id costs a search engine: to mint, to store, to resolve from a search hit, and to find a row by.

Every row in an Infino table has a system-managed `_id`: a 128-bit Snowflake-style id made of a 64-bit millisecond timestamp, a 40-bit random writer id and a 24-bit sequence number. Writers draw their 40 bits at random when they start, so they mint ids without coordinating with each other.

Everything measured is the engine's own code, at the commit pinned in `Cargo.toml`. Ids are minted by its `IdGenerator`, every superfile is built by its `SuperfileBuilder`, and hits are resolved to ids by its `SuperfileReader`. The bench measures:

| table | what it measures |
|---|---|
| Minting | ids per second with 1 writer and with one writer per logical CPU, each with its own generator: one id at a time through `next_id`, and in batches of 1,024 through `reserve_range`, the way an append reserves its ids with one clock read and fills them in |
| One generator shared by every thread | appends through one supertable handle share its generator behind a lock: the lock taken for every id, and once per batch of 1,024 |
| Bytes per id | one superfile of 1M ids from one writer: the Parquet `_id` column and the packed id sidecar, against the 16 raw bytes |
| Resolving k hits to ids | turning the row numbers of a search's top k hits into `_id`s, from the Parquet `_id` column and from the id sidecar, for k = 10, 100 and 1,000 |
| Finding a row by id | what a delete or an update does first: skip every superfile whose id range cannot hold the id, and scan the `_id` column of the rest, in a table of 100 superfiles of 100,000 rows written by 1 and by 4 concurrent writers |

A test checks that both ways of minting yield distinct ids in increasing order.

## Numbers

No result has been published yet for this version of the bench. The tables will be copied verbatim from the first file `run.sh` writes under `results/`.

## Running it

```sh
cd idbench
cargo run --release           # the full run, a few minutes on 8 vCPUs after the build
cargo run --release -- quick  # smaller sizes, under a minute
cargo run --release -- mint   # only the minting tables
```

The toolchain is the repository's, from `rust-toolchain.toml`, i.e. the same one the engine pins. Each run prints its tables as markdown. The absolute numbers depend on the machine. Writer counts follow the machine's logical CPU count.

To publish a result, commit first and then run:

```sh
./run.sh gcp-c3-highcpu-8
```

It refuses to run with uncommitted changes under `idbench/`. It writes `results/<date>-<machine>.md` with the tables and what produced them: the CPU, memory, OS, `rustc`, the exact Infino commit from `Cargo.lock`, this repository's commit, and the load average when the run started. A starting load well above zero means something else was using the machine, or had just stopped. The tables in this README are copied verbatim from such a file.

To measure a later Infino, change the `rev` of the `infino` dependency in `Cargo.toml`, run `cargo update -p infino`, commit, and run `./run.sh` again.
