# idbench results, gcp-c3-highcpu-8

| | |
|---|---|
| started | 2026-10-23 07:46 UTC |
| machine | gcp-c3-highcpu-8 |
| cpu | Intel(R) Xeon(R) Platinum 8481C CPU @ 2.70GHz, 8 logical cpus |
| memory | 15 GiB |
| os | Rocky Linux 9.8 (Blue Onyx), kernel 5.14.0-687.52.1+2.1.el9_8_ciq.x86_64 |
| rustc | rustc 1.95.0 (59807616e 2026-04-14) |
| infino | `git+https://github.com/infino-ai/infino?rev=c5a80fa9#c5a80fa9800d5d0004ae0d0ff1493923e3707d66` |
| retrievalbench | `d06f0a8cee3cfd1a139c321c594a975a053da646` |
| command | `./run.sh gcp-c3-highcpu-8` |
| load average at start (1, 5, 15 min) | 0.01, 0.50, 0.94 |

cpus: 8, ids per thread: 5000000, median of 5 runs

## Minting one id at a time

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

## Minting in batches of 1024

| scheme | 1 writer | 8 writers | ns per id, 1 writer |
|---|---:|---:|---:|
| auto-increment | 2.82 B/s | 9.74 B/s | 0.35 |
| UUIDv4 | 128.0 M/s | 666.5 M/s | 7.81 |
| UUIDv7, a ContextV7 per writer | 40.7 M/s | 185.9 M/s | 24.59 |
| ULID, monotonic Generator per writer | 132.3 M/s | 452.1 M/s | 7.56 |
| Infino-128 | 2.19 B/s | 8.69 B/s | 0.46 |

## One Infino-128 generator shared by 8 threads

| lock taken | ids per second |
|---|---:|
| for every id | 5.7 M/s |
| once per 1024 ids | 4.23 B/s |

## Bytes per id, one superfile of 1000000 docs from one writer

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

## Resolving k hits to ids, one superfile of 1000000 docs

| scheme | k | from the Parquet `_id` column | from the id sidecar | speedup |
|---|---:|---:|---:|---:|
| UUIDv4 | 10 | 162 µs | 0.8 µs | 207× |
| UUIDv4 | 100 | 289 µs | 4.3 µs | 68× |
| UUIDv4 | 1000 | 1079 µs | 43.8 µs | 25× |
| Infino-128 | 10 | 268 µs | 0.6 µs | 473× |
| Infino-128 | 100 | 1337 µs | 2.0 µs | 665× |
| Infino-128 | 1000 | 7897 µs | 16.1 µs | 490× |

## Finding a row by id, 100 superfiles × 100000 docs

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
