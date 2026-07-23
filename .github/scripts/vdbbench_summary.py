#!/usr/bin/env python3
"""Render VectorDBBench result JSON into a GitHub step-summary table.

Usage: vdbbench_summary.py <results_dir> <summary_path>
Env: CLOUD BINDING INFINO_REF MODE VM_SIZE LOCATION NUM_CONC NUM_PER_BATCH
VECTOR_CASE FTS_CASE FTS_DATASET. Exits non-zero if no result JSON is found or
any case failed, so a bad run fails the job.
"""

import glob
import json
import os
import sys

FTS_CASE_ID = 503  # CaseType.FTSBm25Performance
_STATUS = {":)": "✓", "x": "✗ FAILED", "?": "? out-of-range"}  # ResultLabel

# recall@k means different things per bench: vector compares to the exact
# nearest neighbours (retrieval quality); FTS compares to a reference BM25
# top-k (implementation parity, not relevance). Spell that out in the heading.
_RECALL_MEANING = {
    "vector": "recall@k vs exact nearest-neighbors",
    "fts": "recall@k vs reference BM25 top-k",
}


def collect(results_dir: str) -> dict[str, list[dict]]:
    rows: dict[str, list[dict]] = {"vector": [], "fts": []}
    for path in sorted(glob.glob(f"{results_dir}/**/result_*.json", recursive=True)):
        try:
            data = json.load(open(path))
        except (OSError, ValueError) as e:
            print(f"::warning::could not parse {path}: {e}")
            continue
        for r in data.get("results", []):
            m = r.get("metrics", {})
            case_id = r.get("task_config", {}).get("case_config", {}).get("case_id")
            bucket = "fts" if case_id == FTS_CASE_ID else "vector"
            rows[bucket].append(
                {
                    "case": case_id,
                    "status": _STATUS.get(r.get("label"), r.get("label", "?")),
                    "recall": m.get("recall", 0.0),
                    "qps": m.get("qps", 0.0),
                    "p99": m.get("serial_latency_p99", 0.0),
                    "load": m.get("load_duration", 0.0),
                    # Performance cases report inserted_count; max_load_count is
                    # only set for Capacity cases.
                    "count": m.get("inserted_count") or m.get("max_load_count", 0),
                }
            )
    return rows


def table(heading: str, items: list[dict]) -> list[str]:
    lines = [f"### {heading}"]
    if not items:
        return lines + ["**❌ No results — see the uploaded log.**", ""]
    lines.append("| case_id | status | recall@k | QPS | p99 (s) | load (s) | rows |")
    lines.append("|--:|:--|--:|--:|--:|--:|--:|")
    for it in items:
        lines.append(
            f"| {it['case']} | {it['status']} | {it['recall']:.4f} | {it['qps']:.1f} "
            f"| {it['p99']:.4f} | {it['load']:.1f} | {it['count']} |"
        )
    return lines + [""]


def main() -> int:
    results_dir, summary_path = sys.argv[1], sys.argv[2]
    env = os.environ.get
    cloud = env("CLOUD", "?")
    mode = env("MODE", "both")
    binding = (
        "published wheel"
        if env("BINDING") == "published"
        else f"infino ref `{env('INFINO_REF', '?')}`"
    )

    rows = collect(results_dir)
    lines = [
        f"## VectorDBBench — Infino ({cloud})",
        "",
        f"- cloud `{cloud}` · {binding} · mode `{mode}`",
        f"- VM `{env('VM_SIZE') or 'per-cloud default'}` @ `{env('LOCATION') or 'per-cloud default'}`"
        f" · concurrency `{env('NUM_CONC', '?')}` · batch `{env('NUM_PER_BATCH', '?')}`",
        "",
    ]
    if mode in ("vector", "both"):
        heading = f"Vector — {_RECALL_MEANING['vector']} · case `{env('VECTOR_CASE', '?')}`"
        lines += table(heading, rows["vector"])
    if mode in ("fts", "both"):
        heading = f"FTS (BM25) — {_RECALL_MEANING['fts']} · dataset `{env('FTS_DATASET', '?')}`"
        lines += table(heading, rows["fts"])

    with open(summary_path, "a") as fh:
        fh.write("\n".join(lines) + "\n")

    if not rows["vector"] and not rows["fts"]:
        print("::error::VectorDBBench produced no result JSON.")
        return 1
    failed = [r for r in rows["vector"] + rows["fts"] if r["status"] != "✓"]
    if failed:
        print(f"::error::{len(failed)} case(s) failed — see the uploaded log.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
