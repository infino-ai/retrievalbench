# idbench results, gcp-c3-highcpu-8

| | |
|---|---|
| started | 2026-11-05 06:43 UTC |
| machine | gcp-c3-highcpu-8 |
| cpu | Intel(R) Xeon(R) Platinum 8481C CPU @ 2.70GHz, 8 logical cpus |
| memory | 15 GiB |
| os | Rocky Linux 9.8 (Blue Onyx), kernel 5.14.0-687.57.1+2.1.el9_8_ciq.x86_64 |
| rustc | rustc 1.95.0 (59807616e 2026-04-14) |
| infino | `git+https://github.com/infino-ai/infino?rev=c5a80fa9#c5a80fa9800d5d0004ae0d0ff1493923e3707d66` |
| retrievalbench | `78daab89134f3c277f7704d32486416b14e578a7` |
| command | `./run.sh gcp-c3-highcpu-8` |
| load average at start (1, 5, 15 min) | 0.09, 0.65, 0.95 |

cpus: 8, each minting rep runs for 1000 ms, median of 5 reps

## Minting

| how | 1 writer | 8 writers | ns per id, 1 writer |
|---|---:|---:|---:|
| one id at a time (`next_id`) | 1.15 B/s | 3.82 B/s | 0.87 |
| in batches of 1024, as an append reserves them (`reserve_range`) | 1.43 B/s | 4.53 B/s | 0.70 |

Threads in the process during a 8-writer rep: 10 (8 writers, the main thread, and the shared clock's ticker, which wakes once a millisecond).

## One generator shared by 8 threads

| lock taken | ids per second |
|---|---:|
| for every id | 4.7 M/s |
| once per 1024 ids | 3.75 B/s |

## Bytes per id, one superfile of 1000000 docs from one writer

| raw | Parquet `_id` column | packed id sidecar | Parquet `_id` pages | encodings |
|---:|---:|---:|---:|---|
| 16 | 1.08 | 1.29 | 977 | PLAIN RLE |

## Resolving k hits to ids, one superfile of 1000000 docs

| k | from the Parquet `_id` column | from the id sidecar | speedup |
|---:|---:|---:|---:|
| 10 | 214 µs | 0.5 µs | 400× |
| 100 | 1279 µs | 2.1 µs | 599× |
| 1000 | 7901 µs | 16.2 µs | 489× |

## Finding a row by id, 100 superfiles × 100000 docs

| concurrent writers | superfiles scanned | lookup time |
|---:|---:|---:|
| 1 | 1.0 of 100 | 1.9 ms |
| 4 | 4.0 of 100 | 7.5 ms |
