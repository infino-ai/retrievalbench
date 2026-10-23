# idbench

How much a document id scheme costs a search engine, i.e. what it costs to mint, store and look up an id. The bench compares six schemes:

| scheme | layout |
|---|---|
| auto-increment | one counter shared by every writer (an atomic across threads) |
| UUIDv4 | 122 random bits |
| UUIDv7 | 48-bit ms timestamp, then counter and random bits |
| ULID | 48-bit ms timestamp, then 80 random bits |
| Snowflake-64 | Twitter's layout: 41-bit ms timestamp, 10-bit machine id, 12-bit sequence |
| Infino-128 | Infino's `_id`: 64-bit ms timestamp, 40-bit random writer id, 24-bit sequence |

Everything Infino-specific is the engine's own code, at the commit pinned in `Cargo.toml`. Ids are minted by its `IdGenerator`, every superfile is built by its `SuperfileBuilder`, and hits are resolved to ids by its `SuperfileReader`. The other schemes' ids go through the same builder and reader, so the only thing that differs between the rows of two tables is the id.

## Numbers

<!-- NUMBERS -->

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
