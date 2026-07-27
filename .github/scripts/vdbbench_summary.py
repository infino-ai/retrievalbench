#!/usr/bin/env python3
"""Render VectorDBBench result JSON into a GitHub step-summary table.

Usage: vdbbench_summary.py <results_dir> <summary_path>
Env: CLOUD BINDING INFINO_REF INFINO_ENV MODE VM_SIZE LOCATION NUM_CONC
NUM_PER_BATCH CACHE_BUDGET_BYTES VECTOR_CASE FTS_DATASET.

Exits non-zero if no result JSON was produced or a case failed.
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

# Index/query params that shaped the run, per bench. Vector's are the IVF
# knobs; FTS has none (BM25 k1/b are compile-time, the analyzer is fixed).
_PARAMS = {
    "vector": ("nprobe", "n_cent", "rerank_mult"),
    "fts": (),
}

# (column header, metric key, format spec). nDCG stays out of the FTS table:
# only the vector path computes it.
_METRICS = {
    "vector": (
        ("recall", "recall", ".4f"),
        ("nDCG", "ndcg", ".4f"),
        ("max QPS", "qps", ".2f"),
        ("p99 (s)", "serial_latency_p99", ".4f"),
        ("p95 (s)", "serial_latency_p95", ".4f"),
        ("insert (s)", "insert_duration", ".1f"),
        ("optimize (s)", "optimize_duration", ".1f"),
        ("rows", "inserted_count", "d"),
    ),
    "fts": (
        ("recall", "recall", ".4f"),
        ("max QPS", "qps", ".2f"),
        ("p99 (s)", "serial_latency_p99", ".4f"),
        ("p95 (s)", "serial_latency_p95", ".4f"),
        ("insert (s)", "insert_duration", ".1f"),
        ("optimize (s)", "optimize_duration", ".1f"),
        ("bytes/query", "payload_estimated_bytes_per_query", "d"),
        ("rows", "inserted_count", "d"),
    ),
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
            task = r.get("task_config", {})
            case_cfg = task.get("case_config", {})
            bucket = "fts" if case_cfg.get("case_id") == FTS_CASE_ID else "vector"
            rows[bucket].append(
                {
                    "status": _STATUS.get(r.get("label"), r.get("label", "?")),
                    "k": case_cfg.get("k"),
                    "params": task.get("db_case_config", {}),
                    "metrics": r.get("metrics", {}),
                }
            )
    return rows


def cell(value: object, spec: str) -> str:
    """Format a metric, tolerating absent keys in older result JSON."""
    if value is None:
        return "—"
    return f"{value:{spec}}"


def table(bucket: str, items: list[dict]) -> list[str]:
    params = _PARAMS[bucket]
    metrics = _METRICS[bucket]
    cols = [*params, "k", "status", *(head for head, _, _ in metrics)]
    lines = [
        f"| {' | '.join(cols)} |",
        f"|{'|'.join(['--:'] * len(params) + ['--:', ':--'] + ['--:'] * len(metrics))}|",
    ]
    for it in items:
        cells = [str(it["params"].get(p, "—")) for p in params]
        cells.append(str(it["k"] if it["k"] is not None else "—"))
        cells.append(it["status"])
        cells += [cell(it["metrics"].get(key), spec) for _, key, spec in metrics]
        lines.append(f"| {' | '.join(cells)} |")
    return lines + [""]


def concurrency_table(item: dict) -> list[str]:
    """Per-level QPS and latency."""
    m = item["metrics"]
    levels = m.get("conc_num_list") or []
    if not levels:
        return []
    qps = m.get("conc_qps_list") or []
    p99 = m.get("conc_latency_p99_list") or []
    avg = m.get("conc_latency_avg_list") or []
    lines = saturation(levels, qps, p99) + [
        "| clients | QPS | p99 (s) | avg (s) |",
        "|--:|--:|--:|--:|",
    ]
    for i, clients in enumerate(levels):
        lines.append(
            f"| {clients} | {cell(qps[i] if i < len(qps) else None, '.2f')} "
            f"| {cell(p99[i] if i < len(p99) else None, '.4f')} "
            f"| {cell(avg[i] if i < len(avg) else None, '.4f')} |"
        )
    return lines + [""] + chart(levels, qps)


def saturation(levels: list[int], qps: list[float], p99: list[float]) -> list[str]:
    """Where the QPS curve peaks, and the p99 cost at that point."""
    if len(levels) < 2 or len(qps) < len(levels):
        return []
    peak = max(range(len(qps)), key=lambda i: qps[i])
    note = f"QPS peaks at **{levels[peak]} clients** ({qps[peak]:.2f})"
    if len(p99) >= len(levels) and p99[0]:
        note += f", where p99 is {p99[peak] / p99[0]:.1f}× the single-client latency"
    return [f"{note}.", ""]


def chart(levels: list[int], qps: list[float]) -> list[str]:
    """Mermaid line chart of the QPS curve."""
    if len(levels) < 2 or len(qps) < len(levels):
        return []
    top = max(qps)
    return [
        "```mermaid",
        "xychart-beta",
        '    title "QPS vs concurrent clients"',
        f"    x-axis [{', '.join(str(c) for c in levels)}]",
        f'    y-axis "QPS" 0 --> {top * 1.1:.2f}',
        f"    line [{', '.join(f'{q:.2f}' for q in qps)}]",
        "```",
        "",
    ]


def gib(raw: str | None) -> str:
    if not raw:
        return "client default"
    try:
        return f"{int(raw) / 1024**3:.0f} GiB"
    except ValueError:
        return raw


def main() -> int:
    results_dir, summary_path = sys.argv[1], sys.argv[2]
    env = os.environ.get
    cloud = env("CLOUD", "?")
    mode = env("MODE", "?")
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
        f"- VM `{env('VM_SIZE') or 'per-cloud default'}` @ `{env('LOCATION') or 'per-cloud default'}`",
        f"- concurrency `{env('NUM_CONC', '?')}` · insert batch `{env('NUM_PER_BATCH', '?')}`"
        f" · cache budget `{gib(env('CACHE_BUDGET_BYTES'))}`",
        f"- engine env `{env('INFINO_ENV') or 'defaults'}`",
        "",
    ]
    for bucket in ("vector", "fts"):
        if not rows[bucket]:
            continue
        case = env("VECTOR_CASE", "?") if bucket == "vector" else env("FTS_DATASET", "?")
        label = "Vector" if bucket == "vector" else "FTS (BM25)"
        lines += [f"### {label} — `{case}` · {_RECALL_MEANING[bucket]}"]
        lines += table(bucket, rows[bucket])
        for it in rows[bucket]:
            curve = concurrency_table(it)
            if curve:
                lines += ["<details><summary>Concurrency curve</summary>", ""]
                lines += curve + ["</details>", ""]

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
