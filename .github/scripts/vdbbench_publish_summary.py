#!/usr/bin/env python3
"""Summarize the VectorDBBench results a publish step uploaded.

Usage: vdbbench_publish_summary.py <results_dir> <summary_path>

Run context comes from the run_meta_<bench>.json files the bench writes beside
its results, so each leg reports the hardware it actually ran on.

Exits non-zero if no result JSON was found.
"""

import glob
import json
import sys

from vdbbench_summary import LABEL, RECALL_MEANING, collect, table

_CASE_KEY = {"vector": "vector_case", "fts": "fts_dataset"}


def load_meta(results_dir: str) -> dict[str, dict]:
    """Map bench name to the context recorded by that leg."""
    meta = {}
    for path in glob.glob(f"{results_dir}/**/run_meta_*.json", recursive=True):
        try:
            entry = json.load(open(path))
        except (OSError, ValueError) as e:
            print(f"::warning::could not parse {path}: {e}")
            continue
        bench = entry.get("bench")
        if bench:
            meta[bench] = entry
    return meta


def context_line(entry: dict) -> str:
    hardware = " on ".join(x for x in (entry.get("vm_size"), entry.get("cloud")) if x)
    ref = entry.get("infino_ref")
    parts = [
        f"Hardware {hardware}" if hardware else None,
        f"engine infino `{ref}`" if ref else "engine published wheel",
        f"concurrency `{entry['num_concurrency']}`" if entry.get("num_concurrency") else None,
        f"insert batch `{entry['num_per_batch']}`" if entry.get("num_per_batch") else None,
    ]
    return " · ".join(p for p in parts if p)


def main() -> int:
    results_dir, summary_path = sys.argv[1], sys.argv[2]

    rows = collect(results_dir)
    if not rows["vector"] and not rows["fts"]:
        print("::error::no VectorDBBench result JSON to publish.")
        return 1

    meta = load_meta(results_dir)
    lines = [f"## Published {len(rows['vector']) + len(rows['fts'])} case result(s)", ""]

    for bucket in ("vector", "fts"):
        if not rows[bucket]:
            continue
        entry = meta.get(bucket, {})
        case = entry.get(_CASE_KEY[bucket]) or "?"
        lines += [
            f"### {LABEL[bucket]} — `{case}` · {RECALL_MEANING[bucket]}",
            "",
            context_line(entry),
            "",
        ]
        lines += table(bucket, rows[bucket])

    failed = [r for r in rows["vector"] + rows["fts"] if r["status"] != "✓"]
    if failed:
        lines += [f"> {len(failed)} case(s) did not pass; check before deploying.", ""]

    with open(summary_path, "a") as fh:
        fh.write("\n".join(lines) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
