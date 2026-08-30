#!/usr/bin/env python3
"""Commit the in-process harness JSON and render README tables from it.

Copies target/infino-bench/<cell>.json into a run-specific committed
directory, records host/commit/dependency provenance, then rewrites
results/README.md from those files. A refresh is re-run + this script,
never a hand-edited number. Official publication refuses a dirty tree.

Usage:
  scripts/publish_inprocess.py --run-id glove-200-100k \
    --corpus annb:glove-200-angular --docs 100000 \
    --command './scripts/publish_results.sh ...'
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tomllib
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RUN_METADATA = "run.json"


def command_output(*args: str) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def source_tree_changes() -> list[str]:
    lines = command_output("git", "status", "--porcelain").splitlines()
    return [
        line
        for line in lines
        if not line[3:].split(" -> ")[-1].startswith("results/")
    ]


def cpu_name() -> str:
    if sys.platform == "darwin":
        try:
            return command_output("sysctl", "-n", "machdep.cpu.brand_string")
        except (OSError, subprocess.CalledProcessError):
            pass
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.exists():
        for line in cpuinfo.read_text().splitlines():
            if line.startswith("model name") and ":" in line:
                return line.split(":", 1)[1].strip()
    return platform.processor() or "unknown CPU"


def memory_gib() -> float | None:
    if sys.platform == "darwin":
        try:
            return int(command_output("sysctl", "-n", "hw.memsize")) / (1024**3)
        except (OSError, subprocess.CalledProcessError, ValueError):
            pass
    meminfo = Path("/proc/meminfo")
    if meminfo.exists():
        for line in meminfo.read_text().splitlines():
            if line.startswith("MemTotal:"):
                return int(line.split()[1]) / (1024**2)
    return None


def host_info() -> str:
    logical = os_cpu_count()
    ram = memory_gib()
    ram_text = f" · {ram:.0f} GiB RAM" if ram is not None else ""
    return (
        f"Host: {cpu_name()} · {logical} logical cores{ram_text} · "
        f"{platform.system().lower()}/{platform.machine()}"
    )


def os_cpu_count() -> int:
    return os.cpu_count() or 0


def dependency_versions() -> dict[str, str]:
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    wanted = {"faiss", "lancedb", "tantivy", "turbovec", "infino"}
    versions: dict[str, str] = {}
    for package in lock["package"]:
        name = package["name"]
        if name not in wanted:
            continue
        value = package["version"]
        source = package.get("source", "")
        if "#" in source:
            value += f" ({source.rsplit('#', 1)[1][:12]})"
        versions[name] = value
    return versions


def copy_json(src: Path, dst: Path) -> list[Path]:
    if dst.exists():
        shutil.rmtree(dst)
    dst.mkdir(parents=True, exist_ok=True)
    copied: list[Path] = []
    for path in sorted(src.glob("*.json")):
        target = dst / path.name
        shutil.copy2(path, target)
        copied.append(target)
    return copied


def format_duration_ns(value: float) -> str:
    if value >= 1_000_000_000:
        return f"{value / 1_000_000_000:.3g} s"
    if value >= 1_000_000:
        return f"{value / 1_000_000:.3g} ms"
    if value >= 1_000:
        return f"{value / 1_000:.3g} µs"
    return f"{value:.3g} ns"


def format_bytes(value: float) -> str:
    units = ("B", "KiB", "MiB", "GiB", "TiB")
    size = value
    for unit in units:
        if abs(size) < 1024 or unit == units[-1]:
            return f"{size:.3g} {unit}"
        size /= 1024
    raise AssertionError("unreachable")


def format_metric(value: float, subtitle: str, header: str) -> str:
    context = f"{subtitle} {header}".lower()
    if "throughput" in context:
        return f"{value:,.0f}/s"
    if "bandwidth" in context:
        return f"{format_bytes(value)}/s"
    if any(word in context for word in ("rss", "resident", "payload", "bytes")):
        return format_bytes(value)
    if header.lower() == "b/vec":
        return f"{value:.0f}"
    if "recall" in context:
        return f"{value:.4f}"
    if any(
        word in context
        for word in ("latency", "wall", "time", "p50", "p90", "p95", "p99", "open")
    ):
        return format_duration_ns(value)
    return f"{value:.4g}"


def tables_from_json(path: Path) -> str:
    data = json.loads(path.read_text())
    # Keys: anchor|subtitle|label|header → f64. Skip the corpus fingerprint.
    groups: dict[tuple[str, str], dict[str, dict[str, float]]] = defaultdict(
        lambda: defaultdict(dict)
    )
    headers_order: dict[tuple[str, str], list[str]] = defaultdict(list)
    for key, raw in data.items():
        if not isinstance(raw, (int, float)):
            continue
        parts = key.split("|")
        if len(parts) != 4:
            continue
        anchor, subtitle, label, header = parts
        if header in ("Config", "Query", "Op", "Engine", "Shape", "k"):
            continue
        slot = groups[(anchor, subtitle)][label]
        slot[header] = float(raw)
        order = headers_order[(anchor, subtitle)]
        if header not in order:
            order.append(header)
    chunks = [f"### `{path.stem}`\n"]
    for anchor, subtitle in sorted(groups):
        rows = groups[(anchor, subtitle)]
        title = subtitle or anchor
        headers = headers_order[(anchor, subtitle)]
        chunks.append(f"**{title}**\n")
        chunks.append("| Row | " + " | ".join(headers) + " |")
        chunks.append("| --- | " + " | ".join("---" for _ in headers) + " |")
        for label, cols in sorted(rows.items()):
            cells = " | ".join(
                format_metric(cols[h], subtitle, h) if h in cols else "—" for h in headers
            )
            chunks.append(f"| {label} | {cells} |")
        chunks.append("")
    return "\n".join(chunks) + "\n"


def render_readme(dst: Path) -> None:
    parts = [
        "# In-process results\n",
        "Generated by `scripts/publish_inprocess.py` from the committed JSON below.\n",
        "Do not hand-edit numbers. Refresh is re-run + this script.\n",
    ]
    run_dirs = sorted(path for path in dst.iterdir() if path.is_dir())
    for run_dir in run_dirs:
        metadata_path = run_dir / RUN_METADATA
        if not metadata_path.exists():
            continue
        metadata = json.loads(metadata_path.read_text())
        parts.append(f"\n## `{metadata['run_id']}`\n")
        parts.append(f"\n_{metadata['host']}_\n")
        parts.append(
            f"\nCorpus: `{metadata['corpus']}` · docs: `{metadata['docs']:,}` · "
            f"retrievalbench: `{metadata['retrievalbench_commit'][:12]}` · "
            f"Infino: `{metadata['dependencies']['infino']}`\n"
        )
        for path in sorted(run_dir.glob("*.json")):
            if path.name != RUN_METADATA:
                parts.append(tables_from_json(path))
    (dst.parent / "README.md").write_text("".join(parts))


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--src", type=Path, default=ROOT / "target" / "infino-bench")
    p.add_argument("--dst", type=Path, default=ROOT / "results" / "inprocess")
    p.add_argument("--run-id", required=True)
    p.add_argument("--corpus", required=True)
    p.add_argument("--docs", required=True, type=int)
    p.add_argument("--command", required=True)
    p.add_argument(
        "--allow-dirty",
        action="store_true",
        help="development-only: publish results from a dirty source tree",
    )
    args = p.parse_args()
    if not args.src.exists():
        print(f"no metrics at {args.src}; run the harness first")
        return 1
    changes = source_tree_changes()
    if changes and not args.allow_dirty:
        print(
            "refusing to publish from a dirty source tree:\n  " + "\n  ".join(changes),
            file=sys.stderr,
        )
        return 1
    if not re.fullmatch(r"[a-zA-Z0-9._-]+", args.run_id):
        print(f"invalid run id: {args.run_id!r}", file=sys.stderr)
        return 1
    run_dst = args.dst / args.run_id
    copied = copy_json(args.src, run_dst)
    if not copied:
        print(f"no *.json under {args.src}")
        return 1
    metadata = {
        "schema_version": 1,
        "run_id": args.run_id,
        "corpus": args.corpus,
        "docs": args.docs,
        "host": host_info(),
        "retrievalbench_commit": command_output("git", "rev-parse", "HEAD"),
        "dependencies": dependency_versions(),
        "command": args.command,
        "published_at": datetime.now(timezone.utc).isoformat(),
        "dirty": bool(changes),
    }
    (run_dst / RUN_METADATA).write_text(json.dumps(metadata, indent=2) + "\n")
    render_readme(args.dst)
    print(f"copied {len(copied)} files → {run_dst}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
