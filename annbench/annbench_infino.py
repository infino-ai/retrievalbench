#!/usr/bin/env python3
"""ann-benchmarks HDF5 -> infino recall@10 harness (full infino-supported roster).

Reads the standard ann-benchmarks datasets (train/test/neighbors + a metric),
ingests `train` into a local infino table, runs each `test` query, scores
recall@10 vs the precomputed GT. Covers all infino-supported metrics
(euclidean->l2sq, angular->cosine, dot->negdot); jaccard is skipped.
"""

import os
import shutil
import sys
import time
import urllib.request

import h5py
import infino
import numpy as np
import pyarrow as pa

# Full infino-supported roster (metric derived from the name suffix).
DATASETS = [
    "sift-128-euclidean",
    "gist-960-euclidean",
    "mnist-784-euclidean",
    "fashion-mnist-784-euclidean",
    "glove-25-angular",
    "glove-50-angular",
    "glove-100-angular",
    "glove-200-angular",
    "nytimes-256-angular",
    "deep-image-96-angular",
    "lastfm-64-dot",
]
METRIC = {"euclidean": "l2sq", "angular": "cosine", "dot": "negdot"}
K = 10


def metric_of(name):
    return METRIC[name.rsplit("-", 1)[1]]


def download(name, data_dir):
    os.makedirs(data_dir, exist_ok=True)
    p = os.path.join(data_dir, name + ".hdf5")
    if not os.path.exists(p) or os.path.getsize(p) == 0:
        url = f"https://ann-benchmarks.com/{name}.hdf5"
        print(f"  downloading {url}", flush=True)
        req = urllib.request.Request(
            url, headers={"User-Agent": "Mozilla/5.0"}
        )  # host 403s urllib UA
        with urllib.request.urlopen(req, timeout=1200) as r, open(p, "wb") as out:
            shutil.copyfileobj(r, out)
    return p


def build(name, metric, conn_path, data_dir):
    with h5py.File(download(name, data_dir), "r") as f:
        train = np.asarray(f["train"], dtype=np.float32)
        test = np.asarray(f["test"], dtype=np.float32)
        gt = np.asarray(f["neighbors"])[:, :K].astype(np.int64)
    n, dim = train.shape
    if (
        metric == "cosine"
    ):  # angular: normalize for infino's fixed cosine grid (GT rank preserved)
        train /= np.linalg.norm(train, axis=1, keepdims=True) + 1e-9
        test = test / (np.linalg.norm(test, axis=1, keepdims=True) + 1e-9)
    db = infino.connect(conn_path)
    tname = "ann_" + name.replace("-", "_")
    if tname in db.list_tables():
        db.drop_table(tname, purge=True)
    schema = pa.schema(
        [
            pa.field("id", pa.int64(), nullable=False),
            pa.field("emb", pa.list_(pa.float32(), dim), nullable=False),
        ]
    )
    tbl = db.create_table(tname, schema, infino.IndexSpec().vector("emb", dim, metric))
    t0 = time.time()
    B = 100_000
    for s in range(0, n, B):
        e = min(s + B, n)
        tbl.append(
            pa.record_batch(
                [
                    pa.array(range(s, e), pa.int64()),
                    pa.array(
                        [train[i].tolist() for i in range(s, e)],
                        pa.list_(pa.float32(), dim),
                    ),
                ],
                schema=schema,
            )
        )
    ingest = time.time() - t0
    t0 = time.time()
    tbl.optimize()
    opt = time.time() - t0
    print(
        f"  built {name}: n={n} dim={dim} metric={metric} ingest={ingest:.0f}s optimize={opt:.0f}s",
        flush=True,
    )
    # Reopen with a disk cache sized to the index + 10% headroom, matching the
    # engine's own search bench (fresh_supertable_search_cache). Without a cache
    # every query re-reads/re-GETs the whole superfile, inflating latency
    # (catastrophically on object storage) — recall is unaffected either way.
    import glob as _g

    idx = 0
    for d in _g.glob(os.path.join(conn_path, tname + "-*")):
        for f in _g.glob(os.path.join(d, "**", "*"), recursive=True):
            if os.path.isfile(f):
                idx += os.path.getsize(f)
    budget = int(idx * 1.1)
    db = infino.connect(
        conn_path,
        cache_dir=os.path.join(conn_path, "_scache_" + tname),
        cache_budget_bytes=budget,
    )
    tbl = db.open_table(tname)
    for i in range(min(200, len(test))):
        tbl.vector_search(
            "emb", test[i].tolist(), K, projection=["id"]
        )  # warm the cache
    print(
        f"  search cache={budget / 1e9:.2f}GB (index {idx / 1e9:.2f}GB + 10%), warmed",
        flush=True,
    )
    return tbl, test, gt


def score(tbl, test, gt, tag, rerank_mult=None):
    opts = {"rerank_mult": rerank_mult} if rerank_mult else {}
    hits, lat = 0, []
    for q in range(len(test)):
        t = time.time()
        res = tbl.vector_search("emb", test[q].tolist(), K, projection=["id"], **opts)
        lat.append((time.time() - t) * 1000)
        hits += len(set(res.column("id").to_pylist()) & {int(x) for x in gt[q]})
    recall = hits / (len(test) * K)
    lat = np.array(lat)
    print(
        f"    {tag}: recall@{K}={recall:.4f}  p95={np.percentile(lat, 95):.1f}ms  "
        f"p99={np.percentile(lat, 99):.1f}ms  [rm={rerank_mult}]",
        flush=True,
    )
    return recall


def main():
    conn = sys.argv[1] if len(sys.argv) > 1 else "/tmp/ann-infino"
    data_dir = sys.argv[2] if len(sys.argv) > 2 else "/tmp/ann-data"
    for name in sys.argv[3:] or DATASETS:
        metric = metric_of(name)
        print(f"=== {name} ({metric}) ===", flush=True)
        try:
            tbl, test, gt = build(name, metric, conn, data_dir)
            score(tbl, test, gt, "default")
            score(tbl, test, gt, "rm256", rerank_mult=256)
        except Exception as e:  # noqa: BLE001 — keep going across the roster
            print(f"    FAILED {name}: {e}", flush=True)
    print("ANNBENCH_DONE", flush=True)


if __name__ == "__main__":
    main()
