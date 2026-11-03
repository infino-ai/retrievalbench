#!/usr/bin/env python3
"""Compare one benchmark run's engines side by side: ingest, every search task, storage and
resource use. A port of the infinohq harness's compare-results.sh, for any set of engines
and any track: every task the track ran gets its own rows, not only the `nested` track's.

Reads, from the run directory, per engine <name>:
  rally-<name>.csv     Rally's CSV report (Metric, Task, Value, Unit)
  storage-<name>.txt   bytes of the engine's data directory, measured on its VM; Rally's
                       store size comes from the index stats, which leave out the Infino
                       table, so this is the number to compare
  metrics-<name>.jsonl CPU and memory samples (sampler.py)
  machine-<name>.json  what the engine ran on
Writes comparison.txt and comparison.json beside them.

usage: compare.py [--price-per-hour=<USD>] <run dir> [engine ...]
  engines default to every rally-*.csv in the run dir; the price, of one engine VM, adds the
  cost of each engine's race
"""
import csv
import glob
import json
import os
import sys

INGEST_TASK = "index-append"
# The AWS harness's ingest rows.
INGEST_METRICS = [
    ("Min Throughput", "docs/s", "higher"),
    ("Median Throughput", "docs/s", "higher"),
    ("Max Throughput", "docs/s", "higher"),
    ("50th percentile latency", "ms", "lower"),
    ("90th percentile latency", "ms", "lower"),
    ("99th percentile latency", "ms", "lower"),
]
# The AWS harness's search rows (it reported them for randomized-term-queries only), for every
# task, with each task's error rate.
SEARCH_METRICS = [
    ("Mean Throughput", "ops/s", "higher"),
    ("50th percentile service time", "ms", "lower"),
    ("90th percentile service time", "ms", "lower"),
    ("99th percentile service time", "ms", "lower"),
    ("error rate", "%", "lower"),
]
# Rally's own housekeeping tasks, which time no query.
ADMIN_TASK_PREFIXES = ("delete-index", "create-index", "check-cluster-health", "refresh",
                       "force-merge", "wait-until", "put-settings", "cluster-health")
BYTES_PER_GB = 1024 ** 3
SAMPLE_INTERVAL_SECS = 5
SECS_PER_HOUR = 3600
# Report layout: the narrowest value column, the unit column, and the space between columns.
COL = 14
UNIT_COL = 8
GAP = 2


def read_csv(path):
    rows = {}
    try:
        with open(path, newline="") as f:
            for row in csv.DictReader(f):
                if row.get("Metric") is None or row.get("Value") in (None, ""):
                    continue
                rows[(row["Metric"], row.get("Task") or "")] = row["Value"]
    except FileNotFoundError:
        pass
    return rows


def read_resources(path):
    samples = []
    try:
        with open(path) as f:
            samples = [json.loads(line) for line in f if line.strip()]
    except FileNotFoundError:
        return None
    if not samples:
        return None

    def col(key):
        return [s[key] for s in samples]

    secs, cpu, cores = col("secs"), col("cpu_percent"), col("process_cores")
    rss, vms = col("db_rss_mb"), col("db_vms_mb")
    return {"samples": len(samples), "duration_hours": sum(secs) / SECS_PER_HOUR,
            "cpu_avg_pct": sum(c * s for c, s in zip(cpu, secs)) / sum(secs), "cpu_max_pct": max(cpu),
            "core_seconds": sum(c * s for c, s in zip(cores, secs)),
            "rss_avg_gb": sum(rss) / len(rss) / 1024, "rss_max_gb": max(rss) / 1024,
            "vms_avg_gb": sum(vms) / len(vms) / 1024, "vms_max_gb": max(vms) / 1024}


def read_text(path):
    try:
        with open(path) as f:
            return f.read().strip()
    except FileNotFoundError:
        return None


def num(v):
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def fmt(v):
    n = num(v)
    return "n/a" if n is None else f"{n:,.2f}"


def winner(values, better):
    known = {k: n for k, v in values.items() if (n := num(v)) is not None}
    if len(known) < 2:
        return ""
    best = (max if better == "higher" else min)(known.values())
    leaders = [k for k, n in known.items() if n == best]
    return leaders[0] if len(leaders) == 1 else "tie"


def gain(a, b, better):
    """How far a is ahead of b, in percent of b."""
    x, y = num(a), num(b)
    if x is None or y is None or y == 0:
        return None
    return (x - y) / y * 100 if better == "higher" else (y - x) / y * 100


def table(out, title, rows, engines):
    out.append("")
    out.append(title)
    out.append("=" * len(title))
    label_w = max(len("Metric"), *(len(r[0]) for r in rows)) + GAP if rows else len("Metric") + GAP
    col_w = max(COL, *(len(e) + GAP for e in engines))
    head = f"{'Metric':<{label_w}}{'Unit':<{UNIT_COL}}" + "".join(f"{e:>{col_w}}" for e in engines) + "  Best"
    out.append(head)
    out.append("-" * len(head))
    for label, unit, values, better in rows:
        out.append(f"{label:<{label_w}}{unit:<{UNIT_COL}}" + "".join(f"{fmt(values[e]):>{col_w}}" for e in engines)
                   + f"  {winner(values, better) if better else ''}")


def main():
    args = sys.argv[1:]
    price = None
    if args and args[0].startswith("--price-per-hour="):
        price = float(args.pop(0).split("=", 1)[1])
    run = args[0]
    engines = args[1:] or sorted(os.path.basename(p)[len("rally-"):-len(".csv")]
                                     for p in glob.glob(os.path.join(run, "rally-*.csv")))
    data = {e: read_csv(os.path.join(run, f"rally-{e}.csv")) for e in engines}
    resources = {e: read_resources(os.path.join(run, f"metrics-{e}.jsonl")) for e in engines}
    machines = {}
    for e in engines:
        text = read_text(os.path.join(run, f"machine-{e}.json"))
        machines[e] = json.loads(text) if text else None
    storage = {e: num(read_text(os.path.join(run, f"storage-{e}.txt"))) for e in engines}

    out = [f"Rally comparison: {os.path.basename(os.path.normpath(run))}", ""]
    for e in engines:
        m = machines[e]
        out.append(f"  {e}: " + (f"{m['engine']} {m['version']}, {m['cpu_model']}, {m['cpu_cores']} cores, "
                                 f"{m['memory_gb']} GB" if m else "machine not recorded"))

    table(out, "Ingest (index-append)",
          [(metric, unit, {e: data[e].get((metric, INGEST_TASK)) for e in engines}, better)
           for metric, unit, better in INGEST_METRICS], engines)

    tasks = sorted({task for e in engines for (metric, task) in data[e]
                    if task and task != INGEST_TASK and not task.startswith(ADMIN_TASK_PREFIXES)})
    errored = {e: [t for t in tasks if (num(data[e].get(("error rate", t))) or 0) > 0] for e in engines}
    search_rows = []
    for task in tasks:
        for metric, unit, better in SEARCH_METRICS:
            values = {e: data[e].get((metric, task)) for e in engines}
            if any(v is not None for v in values.values()):
                search_rows.append((f"{task}: {metric}", unit, values, better))
    table(out, "Search tasks", search_rows, engines)

    table(out, "Storage", [
        ("Data directory on the engine VM (measured)", "GB",
         {e: None if storage[e] is None else storage[e] / BYTES_PER_GB for e in engines}, "lower"),
        ("Rally store size (index stats; not the Infino table)", "GB",
         {e: data[e].get(("Store size", "")) for e in engines}, None),
    ], engines)

    def res(key):
        return {e: (resources[e] or {}).get(key) for e in engines}

    table(out, f"Resources (engine VM and process, sampled every {SAMPLE_INTERVAL_SECS} s over the race)", [
        ("Race duration", "h", res("duration_hours"), "lower"),
        ("Average VM CPU (all cores)", "%", res("cpu_avg_pct"), "lower"),
        ("Peak VM CPU (all cores)", "%", res("cpu_max_pct"), "lower"),
        ("Engine process CPU over the race", "core-s", res("core_seconds"), "lower"),
        ("Average resident memory", "GB", res("rss_avg_gb"), "lower"),
        ("Peak resident memory", "GB", res("rss_max_gb"), "lower"),
        ("Average virtual memory", "GB", res("vms_avg_gb"), "lower"),
        ("Peak virtual memory", "GB", res("vms_max_gb"), "lower"),
    ], engines)

    if price is not None:
        table(out, f"Cost of the race at ${price}/h per engine VM", [
            ("Engine VM over the race", "USD",
             {e: None if resources[e] is None else resources[e]["duration_hours"] * price for e in engines},
             "lower"),
        ], engines)

    # The AWS harness's performance summary, pair by pair: positive is the first engine ahead.
    # A task either engine answered with errors is left out.
    out.append("")
    out.append("Pairwise (positive: the first engine is ahead; tasks with errors left out)")
    for i, a in enumerate(engines):
        for b in engines[i + 1:]:
            out.append(f"  {a} vs {b}:")
            ingest = gain(data[a].get(("Median Throughput", INGEST_TASK)),
                          data[b].get(("Median Throughput", INGEST_TASK)), "higher")
            if ingest is not None:
                out.append(f"    ingest median throughput: {ingest:+.1f}%")
            for task in tasks:
                if task in errored[a] or task in errored[b]:
                    continue
                p50 = gain(data[a].get(("50th percentile service time", task)),
                           data[b].get(("50th percentile service time", task)), "lower")
                if p50 is not None:
                    out.append(f"    {task} p50 service time: {p50:+.1f}%")

    out.append("")
    out.append("Tasks with errors (a refused task shows here; leave it out of every engine's comparison)")
    for e in engines:
        out.append(f"  {e}: {', '.join(errored[e]) if errored[e] else 'none'}")

    text = "\n".join(out) + "\n"
    with open(os.path.join(run, "comparison.txt"), "w") as f:
        f.write(text)
    with open(os.path.join(run, "comparison.json"), "w") as f:
        json.dump({"engines": engines, "machines": machines, "storage_bytes": storage, "resources": resources,
                   "tasks_with_errors": errored,
                   "metrics": {e: {f"{m}|{t}": v for (m, t), v in data[e].items()} for e in engines}},
                  f, indent=2)
    sys.stdout.write(text)


if __name__ == "__main__":
    main()
