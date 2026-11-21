#!/usr/bin/env python3
"""HSEB MS MARCO filtered-vector benchmark for reference engines.

Dataset: https://huggingface.co/datasets/hseb-benchmark/msmarco

The infino side of this comparison is produced by
infino-bench-lab/scripts/hseb.py; this script runs the same corpus,
the same 10K queries, the same per-(k, selectivity) cells and the same
scoring (|retrieved ∩ exact top-k| / k, as HSEB itself scores) against
other engines, and writes the same JSON shape so the reports line up.

  hseb_cmp.py build --engine lancedb --size 1M
  hseb_cmp.py query --engine lancedb --size 1M --out results/x.json

Queries are issued one at a time from one client thread, so p50 is a
single-query latency and qps_1thread is its inverse — the same
methodology as the infino run.

Needs: pyarrow, numpy, orjson, plus the engine under test (measured with
lancedb==0.41.0, chromadb==1.5.9, qdrant-client==1.19.1). Qdrant runs as
a separate server (release binary 1.19.2 was used); point QDRANT_URL at
it (default http://localhost:6333). Raw files: see hseb.py's docstring.
"""

import argparse
import gzip
import json
import os
import platform
import time
from pathlib import Path

import numpy as np
import orjson
import pyarrow as pa

DIMS = {"all-MiniLM-L6-v2": 384, "e5-base-v2": 768, "Qwen3-Embedding-4B": 2560}
SELECTIVITIES = (100, 90, 10)
LAB = Path(os.environ.get("LAB_DATA_DIR", Path.home() / "infino-bench"))


def log(*a):
    print(f"[hseb-cmp {time.strftime('%H:%M:%S')}]", *a, flush=True)


def raw(model, size):
    d = LAB / "hseb" / "raw" / model / size
    return d / "corpus.jsonl.gz", d / "queries.jsonl.gz"


def corpus_batches(path, batch):
    ids, tags, embs = [], [], []
    with gzip.open(path, "rb") as f:
        for line in f:
            d = orjson.loads(line)
            ids.append(d["id"]); tags.append(d["tag"]); embs.append(d["embedding"])
            if len(ids) == batch:
                yield ids, tags, np.asarray(embs, dtype=np.float32)
                ids, tags, embs = [], [], []
    if ids:
        yield ids, tags, np.asarray(embs, dtype=np.float32)


# ─── engines ──────────────────────────────────────────────────────────


class LanceDB:
    """Embedded LanceDB. IVF_HNSW_SQ vector index (cosine), LABEL_LIST
    scalar index on the tag list, filter pushed down as a prefilter."""

    name = "lancedb"

    def __init__(self, root, dim, opts):
        import lancedb
        self.lancedb, self.dim, self.opts = lancedb, dim, opts
        self.db = lancedb.connect(str(root))

    def build(self, corpus):
        schema = pa.schema([
            pa.field("id", pa.int64()),
            pa.field("tag", pa.list_(pa.int64())),
            pa.field("embedding", pa.list_(pa.float32(), self.dim)),
        ])
        tbl = None
        n = 0
        for ids, tags, embs in corpus_batches(corpus, 100_000):
            emb = pa.FixedSizeListArray.from_arrays(pa.array(embs.ravel()), self.dim)
            batch = pa.Table.from_arrays([pa.array(ids, pa.int64()), pa.array(tags, pa.list_(pa.int64())), emb], schema=schema)
            if tbl is None:
                tbl = self.db.create_table("hseb", batch)
            else:
                tbl.add(batch)
            n += len(ids)
            log(f"appended {n:,}")
        t = time.perf_counter()
        tbl.create_index(metric="cosine", vector_column_name="embedding", index_type=self.opts.get("index_type", "IVF_HNSW_SQ"))
        tbl.create_scalar_index("tag", index_type="LABEL_LIST")
        return n, time.perf_counter() - t

    def open(self):
        self.tbl = self.db.open_table("hseb")

    def search(self, vec, k, sel):
        q = self.tbl.search(vec, vector_column_name="embedding").distance_type("cosine").limit(k).select(["id"])
        if "nprobes" in self.opts:
            q = q.nprobes(int(self.opts["nprobes"]))
        if "ef" in self.opts:
            # lance requires ef >= the candidate count it asks HNSW for,
            # which is k * refine_factor when refining
            q = q.ef(max(int(self.opts["ef"]), k * int(self.opts.get("refine_factor", 1))))
        if "refine_factor" in self.opts:
            q = q.refine_factor(int(self.opts["refine_factor"]))
        if sel != 100:
            q = q.where(f"array_has_any(tag, [{sel}])", prefilter=True)
        return q.to_arrow().column("id").to_pylist()


class Chroma:
    """Embedded Chroma (PersistentClient). HNSW, cosine space; the nested
    tags are stored as boolean metadata and filtered with `where`."""

    name = "chroma"

    def __init__(self, root, dim, opts):
        import chromadb
        self.client = chromadb.PersistentClient(path=str(root))
        self.opts = opts

    def build(self, corpus):
        meta = {"hnsw:space": "cosine"}
        for key in ("M", "construction_ef", "search_ef"):
            if key in self.opts:
                meta[f"hnsw:{key}"] = int(self.opts[key])
        col = self.client.create_collection("hseb", metadata=meta)
        step = self.client.get_max_batch_size()
        n = 0
        for ids, tags, embs in corpus_batches(corpus, 100_000):
            metas = [{"t10": 10 in t, "t90": 90 in t} for t in tags]
            sids = [str(i) for i in ids]
            for s in range(0, len(ids), step):
                col.add(ids=sids[s:s + step], embeddings=embs[s:s + step], metadatas=metas[s:s + step])
            n += len(ids)
            log(f"added {n:,}")
        return n, 0.0  # HNSW is built incrementally inside add()

    def open(self):
        self.col = self.client.get_collection("hseb")

    def search(self, vec, k, sel):
        where = None if sel == 100 else {f"t{sel}": True}
        r = self.col.query(query_embeddings=[vec], n_results=k, where=where, include=[])
        return [int(i) for i in r["ids"][0]]


class Qdrant:
    """Qdrant server (release binary, started separately; see QDRANT_URL).
    Mirrors HSEB's own adapter: DOT distance on the normalized vectors,
    integer payload index on the tag list, MatchValue filter, and
    indexed_only=True. HNSW m / ef_construct default to Qdrant's
    (16 / 100) unless given; search ef via --opt hnsw_ef=N. gRPC client."""

    name = "qdrant"

    def __init__(self, root, dim, opts):
        from qdrant_client import QdrantClient, models
        self.m, self.dim, self.opts = models, dim, opts
        self.client = QdrantClient(url=os.environ.get("QDRANT_URL", "http://localhost:6333"),
                                   prefer_grpc=True, timeout=600)
        self.coll = "hseb_" + root.name.replace("-", "_")

    def build(self, corpus):
        m = self.m
        hnsw = {k: int(self.opts[o]) for k, o in (("m", "M"), ("ef_construct", "construction_ef")) if o in self.opts}
        self.client.create_collection(self.coll, vectors_config=m.VectorParams(size=self.dim, distance=m.Distance.DOT),
                                      hnsw_config=m.HnswConfigDiff(**hnsw) if hnsw else None)
        self.client.create_payload_index(self.coll, field_name="tag", field_schema="integer")
        n = 0
        for ids, tags, embs in corpus_batches(corpus, 100_000):
            self.client.upload_collection(self.coll, vectors=embs, payload=[{"tag": t} for t in tags],
                                          ids=ids, batch_size=1024, parallel=4, wait=True)
            n += len(ids)
            log(f"uploaded {n:,}")
        t = time.perf_counter()
        while True:  # wait for the optimizer to finish building HNSW
            info = self.client.get_collection(self.coll)
            # below Qdrant's indexing threshold (~10 MB of vectors) segments stay plain, never HNSW
            small = n * self.dim * 4 < 20 * 2**20
            if info.status == m.CollectionStatus.GREEN and (small or (info.indexed_vectors_count or 0) >= n * 0.99):
                break
            time.sleep(5)
        log(f"indexed {info.indexed_vectors_count:,} vectors, {info.segments_count} segments")
        return n, time.perf_counter() - t

    def open(self):
        m = self.m
        self.params = m.SearchParams(hnsw_ef=int(self.opts["hnsw_ef"]) if "hnsw_ef" in self.opts else None,
                                     indexed_only=True, exact=False)
        self.filters = {s: m.Filter(must=[m.FieldCondition(key="tag", match=m.MatchValue(value=s))]) for s in (10, 90)}

    def search(self, vec, k, sel):
        r = self.client.query_points(self.coll, query=vec, query_filter=self.filters.get(sel),
                                     search_params=self.params, limit=k, with_payload=False)
        return [p.id for p in r.points]


ENGINES = {"lancedb": LanceDB, "chroma": Chroma, "qdrant": Qdrant}


def engine_version(name):
    from importlib.metadata import version
    pkg = {"lancedb": "lancedb", "chroma": "chromadb", "qdrant": "qdrant-client"}[name]
    if name == "qdrant":
        import urllib.request
        srv = json.load(urllib.request.urlopen(os.environ.get("QDRANT_URL", "http://localhost:6333")))
        return "qdrant", f"server {srv.get('version')}, client {version(pkg)}"
    return pkg, version(pkg)


# ─── driver ───────────────────────────────────────────────────────────


def table_root(args):
    tag = "-".join(f"{k}{v}" for k, v in sorted(args.opt.items()) if k in ("index_type", "M", "construction_ef"))
    return LAB / "hseb" / "tables" / f"{args.engine}{'-' + tag if tag else ''}-{args.model}-{args.size}"


def build(args):
    corpus, _ = raw(args.model, args.size)
    root = table_root(args)
    if root.exists():
        raise SystemExit(f"{root} exists; remove it to rebuild")
    root.mkdir(parents=True)
    eng = ENGINES[args.engine](root, DIMS[args.model], args.opt)
    t = time.perf_counter()
    n, index_s = eng.build(corpus)
    total = time.perf_counter() - t
    log(f"{n:,} rows, total {total:.0f}s (index step {index_s:.0f}s)")
    (root / "build.json").write_text(json.dumps({"rows": n, "total_s": total, "index_s": index_s,
                                                 "opts": args.opt, "engine": engine_version(args.engine)}, indent=2))


def query(args):
    _, qpath = raw(args.model, args.size)
    root = table_root(args)
    eng = ENGINES[args.engine](root, DIMS[args.model], args.opt)
    eng.open()
    qs = []
    with gzip.open(qpath, "rb") as f:
        for line in f:
            d = orjson.loads(line)
            qs.append((np.asarray(d["embedding"], dtype=np.float32), {s: d[f"results_{s}_docs"] for s in SELECTIVITIES}))
            if args.limit and len(qs) == args.limit:
                break
    log(f"{len(qs):,} queries, engine opts {args.opt}")
    for vec, _ in qs[: args.warmup]:
        for sel in SELECTIVITIES:
            eng.search(vec, max(args.k), sel)
    report = {"engine": engine_version(args.engine), "opts": args.opt, "model": args.model, "size": args.size,
              "host": {"cpu": platform.processor() or platform.machine(), "cores": os.cpu_count()},
              "build": json.loads((root / "build.json").read_text()), "cells": []}
    for k in args.k:
        for sel in SELECTIVITIES:
            lat, recall, short = [], [], 0
            for vec, gt in qs:
                a = time.perf_counter()
                got = eng.search(vec, k, sel)
                lat.append(time.perf_counter() - a)
                recall.append(len(set(gt[sel][:k]).intersection(got)) / k)
                short += len(got) < k
            ms = np.asarray(lat) * 1e3
            cell = {"k": k, "selectivity": sel, "queries": len(qs), "recall_mean": float(np.mean(recall)),
                    "recall_p5": float(np.percentile(recall, 5)), "under_filled": short,
                    "p50_ms": float(np.percentile(ms, 50)), "p95_ms": float(np.percentile(ms, 95)),
                    "p99_ms": float(np.percentile(ms, 99)), "mean_ms": float(ms.mean()),
                    "qps_1thread": float(len(lat) / ms.sum() * 1e3)}
            report["cells"].append(cell)
            log(f"k={k:<3} sel={sel:>3}%  recall={cell['recall_mean']:.4f}  p50={cell['p50_ms']:.2f}ms  "
                f"p99={cell['p99_ms']:.2f}ms  under-filled={short}")
    if args.out:
        Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        Path(args.out).write_text(json.dumps(report, indent=2))
        log("wrote", args.out)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    for name in ("build", "query"):
        s = sub.add_parser(name)
        s.add_argument("--engine", required=True, choices=ENGINES)
        s.add_argument("--model", default="all-MiniLM-L6-v2", choices=DIMS)
        s.add_argument("--size", default="1M", choices=["1K", "100K", "1M"])
        s.add_argument("--opt", action="append", default=[], metavar="KEY=VALUE",
                       help="engine option, e.g. index_type=IVF_PQ, nprobes=50, ef=200, search_ef=200")
    q = sub.choices["query"]
    q.add_argument("--k", type=int, nargs="+", default=[10, 100])
    q.add_argument("--limit", type=int, default=0)
    q.add_argument("--warmup", type=int, default=200)
    q.add_argument("--out")
    args = p.parse_args()
    args.opt = dict(o.split("=", 1) for o in args.opt)
    build(args) if args.cmd == "build" else query(args)


if __name__ == "__main__":
    main()
