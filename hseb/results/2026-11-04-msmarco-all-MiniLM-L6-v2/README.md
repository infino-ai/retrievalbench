# HSEB MS MARCO, all-MiniLM-L6-v2 — infino vs Qdrant, LanceDB, Chroma

Dataset: [hseb-benchmark/msmarco](https://huggingface.co/datasets/hseb-benchmark/msmarco),
`all-MiniLM-L6-v2` (384-dim, normalized) at 100K and 1M passages (the
1M split is exactly 1,000,000 rows, the largest single corpus in the
dataset). 10,000 queries × k ∈ {10, 100} × filter ∈ {none, tag on 91%
of rows, tag on 11% of rows}. Recall = |returned ∩ exact filtered
top-k| / k against HSEB's ground truth, the same as HSEB's own
`recall_score`. HSEB publishes no results of its own, so every number
here was measured on one host: 8-core x86_64, 62 GB, shared (other
users idle during the runs), 2026-11-04/05.

Latency is single client thread, one query at a time, after 200 warmup
queries — p50 is a per-query latency, not a throughput number.

| engine | version | mode | index / filter |
| --- | --- | --- | --- |
| infino | main `54593948` (Python binding) | embedded | infino vector index, cosine; tags as FTS column |
| Qdrant | 1.19.2 server, client 1.19.1, gRPC | local server | HNSW (m 16, ef_construct 100), DOT, float32; integer payload index, `MatchValue`, `indexed_only` — as HSEB's own adapter |
| LanceDB | 0.41.0 | embedded | IVF_HNSW_SQ, cosine; LABEL_LIST scalar index, prefilter |
| Chroma | 1.5.9 | embedded (PersistentClient) | HNSW, cosine; boolean metadata `where` |

Tuned points: Qdrant `hnsw_ef=256` (top of HSEB's own Qdrant grid);
LanceDB A `nprobes=50 ef=200 refine_factor=3`, B `nprobes=100 ef=400
refine_factor=5` (ef raised to k × refine_factor where lance requires
it). Chroma at 1M ran 500 queries only — its filtered cells take ~0.5–2 s
per query.

## 1M

Recall:

| k | filter | infino | Qdrant | Qdrant ef256 | LanceDB | LanceDB A | LanceDB B | Chroma (500 q) |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 10 | none | 0.9912 | 0.9776 | 0.9895 | 0.9051 | 0.9924 | 0.9956 | 0.9602 |
| 10 | keeps 90% | 0.9762 | 0.9749 | 0.9884 | 0.9109 | 0.9931 | 0.9960 | 0.9602 |
| 10 | keeps 10% | 0.9781 | 0.9911 | 0.9977 | 0.9611 | 0.9987 | 0.9994 | 0.9882 |
| 100 | none | 0.9247 | 0.9634 | 0.9855 | 0.9667 | 0.9875 | 0.9934 | 0.9110 |
| 100 | keeps 90% | 0.9792 | 0.9580 | 0.9834 | 0.9674 | 0.9880 | 0.9937 | 0.9113 |
| 100 | keeps 10% | 0.9999 | 0.9765 | 0.9945 | 0.9797 | 0.9952 | 0.9980 | 0.9422 |

p50 latency (ms):

| k | filter | infino | Qdrant | Qdrant ef256 | LanceDB | LanceDB A | LanceDB B | Chroma (500 q) |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 10 | none | 37.0 | 1.5 | 2.3 | 2.8 | 4.0 | 4.5 | 1.1 |
| 10 | keeps 90% | 125.9 | 1.7 | 2.7 | 12.6 | 14.1 | 14.7 | 1897.5 |
| 10 | keeps 10% | 59.4 | 1.8 | 2.9 | 15.6 | 20.3 | 24.1 | 449.1 |
| 100 | none | 41.5 | 2.4 | 3.2 | 3.5 | 5.9 | 7.0 | 1.3 |
| 100 | keeps 90% | 143.1 | 2.5 | 3.5 | 13.4 | 16.1 | 17.2 | 1854.5 |
| 100 | keeps 10% | 98.1 | 2.6 | 3.7 | 18.7 | 23.9 | 28.1 | 447.4 |

Build, 1M rows: infino 111 s (75 s load + append, 36 s optimize);
Qdrant 232 s (172 s upload, 60 s HNSW optimizer); LanceDB 214 s (149 s
index); Chroma 1081 s. On disk: infino 1.6 GB after `gc()` (2.5 GB
before — pre-optimize segments and old manifests; live = one 857 MB data
superfile + six vector-index superfiles totalling 847 MB), Qdrant 2.1 GB,
LanceDB 2.1 GB, Chroma 1.9 GB.

## 100K

Recall:

| k | filter | infino | Qdrant | Qdrant ef256 | LanceDB | LanceDB A | LanceDB B | Chroma |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 10 | none | 0.9146 | 0.9890 | 0.9967 | 0.8917 | 0.9958 | 0.9984 | 0.9743 |
| 10 | keeps 90% | 0.9855 | 0.9872 | 0.9962 | 0.8975 | 0.9961 | 0.9985 | 0.9764 |
| 10 | keeps 10% | 1.0000 | 1.0000 | 1.0000 | 0.9624 | 0.9997 | 0.9999 | 0.9952 |
| 100 | none | 0.9698 | 0.9677 | 0.9918 | 0.9675 | 0.9903 | 0.9959 | 0.9165 |
| 100 | keeps 90% | 0.9999 | 0.9618 | 0.9901 | 0.9687 | 0.9909 | 0.9962 | 0.9190 |
| 100 | keeps 10% | 0.9999 | 1.0000 | 1.0000 | 0.9840 | 0.9982 | 0.9996 | 0.9546 |

p50 latency (ms):

| k | filter | infino | Qdrant | Qdrant ef256 | LanceDB | LanceDB A | LanceDB B | Chroma |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 10 | none | 9.8 | 1.3 | 1.9 | 2.6 | 3.7 | 4.1 | 1.1 |
| 10 | keeps 90% | 23.4 | 1.4 | 2.0 | 3.4 | 4.6 | 5.1 | 167.7 |
| 10 | keeps 10% | 18.1 | 1.3 | 1.3 | 3.9 | 7.7 | 10.7 | 45.8 |
| 100 | none | 13.3 | 2.1 | 2.7 | 2.9 | 5.1 | 6.4 | 1.3 |
| 100 | keeps 90% | 31.1 | 2.2 | 2.9 | 3.9 | 6.1 | 8.3 | 168.1 |
| 100 | keeps 10% | 22.3 | 2.2 | 2.2 | 6.2 | 10.6 | 14.1 | 46.0 |

## Reading

- **Latency is infino's gap.** At 1M, LanceDB B beats infino's recall
  in 5 of 6 cells at 2.5–9× lower p50; Qdrant ef256 beats it in 4 of 6
  at 16–47× lower p50. infino's clear win is k=100 on the 11% filter
  (0.9999 vs 0.995–0.998); at k=10 unfiltered it ties (0.991 vs
  0.990–0.996).
- **Broad filters cost infino the most.** The 91% filter is close to no
  filter, yet infino's p50 goes 37 → 126 ms (3.4×) at k=10; Qdrant goes
  1.5 → 1.7 ms. The filtered path does far more work than a filter that
  drops 9% of rows warrants.
- **Unfiltered k=100 recall (0.925 at 1M) is infino's weakest cell** —
  plain ANN recall, below every tuned competitor (0.986–0.993).
- Chroma's filtered search degrades to a near-scan (0.45–1.9 s at 1M).
- Defaults matter: LanceDB's default k=10 recall is ~0.90; with search
  parameters raised it leads on recall.

## Reproduce

```
export LAB_DATA_DIR=~/infino-bench
# infino:   infino-bench-lab/scripts/hseb.py build|query --size 1M --out ...
# others:   hseb/hseb_cmp.py build --engine {qdrant,lancedb,chroma} --size 1M
#           hseb/hseb_cmp.py query --engine ... --size 1M [--opt K=V] --out ...
# Qdrant:   release binary, QDRANT__STORAGE__STORAGE_PATH=..., QDRANT_URL defaults to localhost:6333
hseb/report.py NAME=path.json ...   # these tables
```
