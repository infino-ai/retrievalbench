#!/usr/bin/env python3
"""Record one engine's answer to each of a track's searches, after its race, so compare.py
can check that every engine answered the same. Rally times a request and counts it a success
whatever it returns; this records what came back.

The track is rendered as Rally renders it (Rally's own template loader, so run this with the
Python of the Rally install), and each search operation is sent once, as the track writes
it, with its total counted exactly. Recorded per search: the exact total, the sort values
of the page a sorted search returns, and every aggregation's buckets (keys and counts) and
values.

usage: answers.py <rally.ini> <track> <track params JSON> <host:port> <out .json>
"""
import configparser
import glob
import json
import os
import sys
import urllib.error
import urllib.request

from esrally.track.loader import render_template_from_file

SEARCH_TYPES = ("search", "paginated-search", "scroll-search")
TIMEOUT_SECS = 600


def track_file(rally_ini, track):
    ini = configparser.ConfigParser(interpolation=None)
    ini.read(rally_ini)
    # Rally's own config names its directory as ${CONFIG_DIR}.
    root = ini.get("node", "root.dir").replace("${CONFIG_DIR}", os.path.dirname(os.path.abspath(rally_ini)))
    found = sorted(glob.glob(os.path.join(root, "tracks", "*", track, "track.json")))
    if not found:
        sys.exit(f"no track {track} under {root}/tracks")
    return found[0]


def aggs(result):
    """Each aggregation's buckets as (key, count, sub-aggregations) and its values, recursively."""
    out = {}
    for name, agg in (result or {}).items():
        if not isinstance(agg, dict):
            continue
        if "buckets" in agg:
            buckets = agg["buckets"]
            items = buckets.items() if isinstance(buckets, dict) else ((b.get("key"), b) for b in buckets)
            out[name] = [[key, b.get("doc_count"), aggs({k: v for k, v in b.items() if isinstance(v, dict)})]
                         for key, b in items]
        else:
            out[name] = {k: v for k, v in agg.items() if k in ("value", "values", "doc_count", "count", "min", "max", "avg", "sum")}
            sub = aggs({k: v for k, v in agg.items() if isinstance(v, dict) and k not in ("values", "meta")})
            if sub:
                out[name]["aggs"] = sub
    return out


def answer(host, op):
    body = dict(op.get("body") or {})
    body["track_total_hits"] = True
    params = "&".join(f"{k}={v}" for k, v in (op.get("request-params") or {}).items() if k != "track_total_hits")
    url = f"http://{host}/{op.get('index', '_all')}/_search" + (f"?{params}" if params else "")
    req = urllib.request.Request(url, data=json.dumps(body).encode(), method="POST",
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT_SECS) as r:
            d = json.load(r)
    except urllib.error.HTTPError as e:
        return {"error": e.code}
    except OSError as e:
        return {"error": str(e)}
    out = {"total": d["hits"]["total"]["value"]}
    if body.get("sort"):
        out["sort"] = [h.get("sort") for h in d["hits"]["hits"]]
    if "aggregations" in d:
        out["aggs"] = aggs(d["aggregations"])
    return out


def main():
    rally_ini, track, params, host, out = sys.argv[1:6]
    rendered = json.loads(render_template_from_file(track_file(rally_ini, track), json.loads(params)))
    answers = {op["name"]: answer(host, op) for op in rendered.get("operations", [])
               if op.get("operation-type") in SEARCH_TYPES}
    with open(out, "w") as f:
        json.dump(answers, f, indent=1, sort_keys=True)
    print(f"answers to {len(answers)} searches in {out}")


if __name__ == "__main__":
    main()
