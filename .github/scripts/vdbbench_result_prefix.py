#!/usr/bin/env python3
"""Print the results-tree prefix a VectorDBBench result file belongs under.

Usage: vdbbench_result_prefix.py <result.json>

The full-text-search page reads only results/FullTextSearch/<backend>/, so FTS
results have to land there. Everything else sits under the backend directory,
where the generic result loader finds it.
"""

import json
import sys

from vdbbench_summary import FTS_CASE_ID


def prefix(path: str) -> str:
    results = json.load(open(path)).get("results", [])
    if not results:
        msg = f"{path}: no case results to place"
        raise ValueError(msg)

    backends = {r.get("task_config", {}).get("db") for r in results}
    backends.discard(None)
    if len(backends) != 1:
        msg = f"{path}: expected one backend, found {sorted(backends)}"
        raise ValueError(msg)
    backend = backends.pop()

    case_ids = {r.get("task_config", {}).get("case_config", {}).get("case_id") for r in results}
    if FTS_CASE_ID in case_ids:
        if len(case_ids) != 1:
            msg = f"{path}: mixes FTS and other cases, so it has no single home"
            raise ValueError(msg)
        return f"FullTextSearch/{backend}"
    return backend


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        sys.stderr.write(f"usage: {argv[0]} <result.json>\n")
        return 2
    try:
        print(prefix(argv[1]))
    except (OSError, ValueError) as error:
        sys.stderr.write(f"::error::{error}\n")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
