#!/usr/bin/env python3
"""Render VectorDBBench result JSON into a GitHub step-summary table.

Usage: vdbbench_summary.py <results_dir> <summary_path>
Reads CLOUD, INFINO_REF, MODE from the environment. Exits non-zero if no
result JSON is found, so an empty run fails the job.
"""

import glob
import json
import os
import sys

FTS_CASE_ID = 503  # CaseType.FTSBm25Performance


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
                    "recall": m.get("recall", 0.0),
                    "qps": m.get("qps", 0.0),
                    "p99": m.get("serial_latency_p99", 0.0),
                    "load": m.get("load_duration", 0.0),
                    "count": m.get("max_load_count", 0),
                }
            )
    return rows


def table(title: str, items: list[dict]) -> list[str]:
    lines = [f"### {title}"]
    if not items:
        return lines + ["**❌ No results — see the uploaded log.**", ""]
    lines.append("| case_id | recall | QPS | p99 (s) | load (s) | rows |")
    lines.append("|--:|--:|--:|--:|--:|--:|")
    for it in items:
        lines.append(
            f"| {it['case']} | {it['recall']:.4f} | {it['qps']:.1f} "
            f"| {it['p99']:.4f} | {it['load']:.1f} | {it['count']} |"
        )
    return lines + [""]


def main() -> int:
    results_dir, summary_path = sys.argv[1], sys.argv[2]
    cloud = os.environ.get("CLOUD", "?")
    infino_ref = os.environ.get("INFINO_REF", "?")
    mode = os.environ.get("MODE", "both")

    rows = collect(results_dir)
    lines = [
        f"## VectorDBBench — Infino ({cloud})",
        "",
        f"- cloud `{cloud}` · infino ref `{infino_ref}` · mode `{mode}`",
        "",
    ]
    if mode in ("vector", "both"):
        lines += table("Vector", rows["vector"])
    if mode in ("fts", "both"):
        lines += table("FTS (BM25)", rows["fts"])

    with open(summary_path, "a") as fh:
        fh.write("\n".join(lines) + "\n")

    if not rows["vector"] and not rows["fts"]:
        print("::error::VectorDBBench produced no result JSON.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
