#!/usr/bin/env bash
# Race engines that are already running (on this machine or anywhere this machine reaches),
# one after another, with a local install of the Rally fork, then compare them. The same
# races as benchmark.sh without the VMs: no CPU or memory samples, and no storage figure
# unless you write storage-<name>.txt (bytes) into the results directory yourself.
#
# Usage: opensearch-rally/local.sh <name>=<host:port>[,<database type>] ...
#   database type: opensearch (the default) or elasticsearch
#   e.g. opensearch-rally/local.sh opensearch=localhost:9201 opensearch-infino=localhost:9200
# Environment: as race.sh (ESRALLY, TRACK, TRACK_PARAMS, ...), plus
#   RALLY_DISTRIBUTION_VERSION (default 9.1.3, the version benchmark.sh passes) and
#   INFINO_EXCLUDE_TASKS (added to EXCLUDE_TASKS for names starting opensearch-infino).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RALLY_DISTRIBUTION_VERSION=${RALLY_DISTRIBUTION_VERSION:-9.1.3}
EXCLUDE_TASKS=${EXCLUDE_TASKS:-indices-stats}
INFINO_EXCLUDE_TASKS=${INFINO_EXCLUDE_TASKS:-}
OUT=$HERE/results/local-$(date -u +%Y%m%d-%H%M%S)
[ $# -gt 0 ] || { echo "usage: $0 <name>=<host:port>[,<database type>] ..." >&2; exit 1; }

names=()
for arg in "$@"; do
  name=${arg%%=*}
  target=${arg#*=}
  db=opensearch
  case "$target" in *,*) db=${target#*,}; target=${target%%,*} ;; esac
  exclude=$EXCLUDE_TASKS
  case "$name" in
    opensearch-infino*) [ -n "$INFINO_EXCLUDE_TASKS" ] && exclude="$exclude,$INFINO_EXCLUDE_TASKS" ;;
  esac
  EXCLUDE_TASKS=$exclude "$HERE/race.sh" "$name" "$target" "$db" "$RALLY_DISTRIBUTION_VERSION" "$OUT" \
    || echo "race $name failed; continuing"
  names+=("$name")
done
python3 "$HERE/compare.py" "$OUT" "${names[@]}"
echo "results in $OUT"
