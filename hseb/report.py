#!/usr/bin/env python3
"""Side-by-side markdown for HSEB result JSONs.

  report.py NAME=path.json [NAME=path.json ...]

Accepts both the infino reports (infino-bench-lab/scripts/hseb.py) and
the reference-engine reports (hseb_cmp.py); they share the cell shape.
One table per metric, rows are (k, filter), columns are the named runs.
"""

import json
import sys


def main():
    runs = []
    for arg in sys.argv[1:]:
        name, path = arg.split("=", 1)
        try:
            runs.append((name, json.load(open(path))))
        except FileNotFoundError:
            print(f"<!-- missing: {path} -->")
    if not runs:
        return
    keys = sorted({(c["k"], c["selectivity"]) for _, r in runs for c in r["cells"]}, key=lambda x: (x[0], -x[1]))
    cell = {(n, c["k"], c["selectivity"]): c for n, r in runs for c in r["cells"]}
    names = [n for n, _ in runs]
    flt = {100: "none (100%)", 90: "keeps 90%", 10: "keeps 10%"}
    for title, field, fmt in (("recall", "recall_mean", "{:.4f}"), ("p50 latency (ms)", "p50_ms", "{:.1f}"),
                              ("p99 latency (ms)", "p99_ms", "{:.1f}")):
        print(f"\n**{title}**\n")
        print("| k | filter | " + " | ".join(names) + " |")
        print("| ---: | --- | " + " | ".join("---:" for _ in names) + " |")
        for k, s in keys:
            vals = [fmt.format(cell[(n, k, s)][field]) if (n, k, s) in cell else "—" for n in names]
            print(f"| {k} | {flt[s]} | " + " | ".join(vals) + " |")
    print("\n**build**\n")
    for n, r in runs:
        b = r["build"]
        if "optimize_s" in b:
            print(f"- {n}: {b['rows']:,} rows, load+append {b['load_and_append_s']:.0f}s, optimize {b['optimize_s']:.0f}s")
        else:
            print(f"- {n}: {b['rows']:,} rows, total {b['total_s']:.0f}s (index step {b['index_s']:.0f}s), opts {b['opts']}")
    under = [(n, c["k"], c["selectivity"], c["under_filled"]) for n, r in runs for c in r["cells"] if c["under_filled"]]
    for n, k, s, u in under:
        print(f"- {n}: {u} queries returned fewer than k={k} rows at {flt[s]}")


if __name__ == "__main__":
    main()
