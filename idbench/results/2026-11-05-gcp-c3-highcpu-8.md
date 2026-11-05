# idbench results, gcp-c3-highcpu-8

| | |
|---|---|
| started | 2026-11-05 06:29 UTC |
| machine | gcp-c3-highcpu-8 |
| cpu | Intel(R) Xeon(R) Platinum 8481C CPU @ 2.70GHz, 8 logical cpus |
| memory | 15 GiB |
| os | Rocky Linux 9.8 (Blue Onyx), kernel 5.14.0-687.57.1+2.1.el9_8_ciq.x86_64 |
| rustc | rustc 1.95.0 (59807616e 2026-04-14) |
| infino | `git+https://github.com/infino-ai/infino?rev=c5a80fa9#c5a80fa9800d5d0004ae0d0ff1493923e3707d66` |
| retrievalbench | `db5a0ae2b8de988bb77749c554ee43d8c1b641a9` |
| command | `./run.sh gcp-c3-highcpu-8` |
| load average at start (1, 5, 15 min) | 0.08, 1.23, 1.29 |

cpus: 8, ids per writer: 5000000, median of 5 runs

## Minting

| how | 1 writer | 8 writers | ns per id, 1 writer |
|---|---:|---:|---:|
| one id at a time (`next_id`) | 986.0 M/s | 3.84 B/s | 1.01 |
| in batches of 1024, as an append reserves them (`reserve_range`) | 1.42 B/s | 5.67 B/s | 0.70 |

## One generator shared by 8 threads

| lock taken | ids per second |
|---|---:|
| for every id | 4.6 M/s |
| once per 1024 ids | 3.91 B/s |

## Bytes per id, one superfile of 1000000 docs from one writer

| raw | Parquet `_id` column | packed id sidecar | Parquet `_id` pages | encodings |
|---:|---:|---:|---:|---|
| 16 | 1.08 | 1.29 | 977 | PLAIN RLE |

## Resolving k hits to ids, one superfile of 1000000 docs

| k | from the Parquet `_id` column | from the id sidecar | speedup |
|---:|---:|---:|---:|
| 10 | 271 µs | 0.6 µs | 472× |
| 100 | 1340 µs | 2.1 µs | 640× |
| 1000 | 7962 µs | 16.3 µs | 488× |

## Finding a row by id, 100 superfiles × 100000 docs

| concurrent writers | superfiles scanned | lookup time |
|---:|---:|---:|
| 1 | 1.0 of 100 | 1.9 ms |
| 4 | 4.0 of 100 | 7.6 ms |
