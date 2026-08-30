#!/usr/bin/env python3
"""Turn a clickbench-cloud run log into clickbench/results/infino/<machine>.json.

The workflow tees the VM sweep to /tmp/clickbench.log. Each query line is:

    <q> <t1> <t2> <t3> <hot>

where t1 is cold, t2/t3 are the warm tries, and hot is min(t2, t3). The
committed ClickBench file stores [t1, t2, t3] per query (43 queries).

Usage:
  scripts/ingest_clickbench_log.py \\
      --log /tmp/clickbench.log \\
      --machine c8g.metal-48xl \\
      --out clickbench/results/infino/c8g.metal-48xl.json
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import date
from pathlib import Path

N_QUERIES = 43
LOAD_RE = re.compile(r"LOAD_TIME=([0-9.]+)s")
SIZE_RE = re.compile(r"DATA_SIZE_BYTES=([0-9]+)")
# Query id, then four floats (cold, t2, t3, hot). Hot is ignored; JSON keeps the three tries.
ROW_RE = re.compile(
    r"^(\d+)\s+([0-9.]+)\s+([0-9.]+)\s+([0-9.]+)\s+([0-9.]+)\s*$"
)


def parse_log(text: str) -> tuple[list[list[float]], float | None, int | None]:
    rows: dict[int, list[float]] = {}
    for line in text.splitlines():
        m = ROW_RE.match(line.strip())
        if m:
            q = int(m.group(1))
            t1, t2, t3 = (float(m.group(2)), float(m.group(3)), float(m.group(4)))
            rows[q] = [t1, t2, t3]
    ordered = [rows[i] for i in range(1, N_QUERIES + 1) if i in rows]
    load = None
    m = LOAD_RE.search(text)
    if m:
        load = float(m.group(1))
    size = None
    m = SIZE_RE.search(text)
    if m:
        size = int(m.group(1))
    return ordered, load, size


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--log", required=True, type=Path)
    p.add_argument("--machine", required=True)
    p.add_argument("--out", required=True, type=Path)
    p.add_argument("--date", default=date.today().isoformat())
    p.add_argument("--system", default="infino")
    p.add_argument("--infino-ref", required=True)
    p.add_argument("--comment", default="")
    args = p.parse_args()
    text = args.log.read_text()
    result, load, size = parse_log(text)
    if len(result) != N_QUERIES:
        print(
            f"expected {N_QUERIES} query rows, got {len(result)}",
            file=sys.stderr,
        )
        return 1
    payload = {
        "system": args.system,
        "date": args.date,
        "machine": args.machine,
        "cluster_size": 1,
        "proprietary": "no",
        "hardware": "cpu",
        "tuned": "no",
        "tags": ["Rust", "column-oriented", "embedded", "stateless", "Parquet"],
        "infino_ref": args.infino_ref,
        "comment": args.comment,
        "load_time": load,
        "data_size": size,
        "result": result,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2) + "\n")
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
