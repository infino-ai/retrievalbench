#!/usr/bin/env bash
# One Rally race against one engine, as the AWS harness ran it (run-benchmarks.sh): the
# benchmark-only pipeline (no cluster provisioning), the track's own data and operations,
# --on-error=continue so a refused task does not stop the rest, and a CSV report. Used on
# the runner VM by benchmark.sh, and on your own machine by local.sh.
#
# Usage: race.sh <name> <host:port> <database type> <distribution version> <out dir>
#   database type: elasticsearch | opensearch (the fork's database_type client option)
#   distribution version: Rally picks the track's revision by it; the AWS harness passed the
#     Elasticsearch version for every engine, and benchmark.sh does the same
# Environment: ESRALLY (the esrally binary), TRACK, TRACK_REPOSITORY, CHALLENGE,
#   TRACK_PARAMS (JSON), EXCLUDE_TASKS, INCLUDE_TASKS, TEST_MODE=1, PARALLEL=1 (other races
#   run beside this one: give it its own Rally home and data cache, as the AWS harness did,
#   and leave the other esrally processes running).
set -euo pipefail

NAME=$1
TARGET=$2
DATABASE_TYPE=$3
DIST_VERSION=$4
OUT=$5
ESRALLY=${ESRALLY:-esrally}
TRACK=${TRACK:?set TRACK}
TRACK_PARAMS=${TRACK_PARAMS:-'{"number_of_replicas":0}'}
EXCLUDE_TASKS=${EXCLUDE_TASKS:-indices-stats}

args=(race
  "--track=$TRACK"
  --pipeline=benchmark-only
  "--target-hosts=$TARGET"
  "--track-params=$TRACK_PARAMS"
  "--client-options=database_type:$DATABASE_TYPE"
  "--distribution-version=$DIST_VERSION"
  --on-error=continue
  --report-format=csv
  "--report-file=$OUT/rally-$NAME.csv")
if [ "${PARALLEL:-0}" = 1 ]; then
  # Rally reads its config from $RALLY_HOME/.rally; this race's copy points the track and
  # data directories under its own home.
  export RALLY_HOME="$OUT/home-$NAME"
  mkdir -p "$RALLY_HOME/.rally"
  sed "s#^\(root.dir\|src.root.dir\|local.dataset.cache\) = .*/\.rally/benchmarks#\1 = $RALLY_HOME/.rally/benchmarks#" \
    "$HOME/.rally/rally.ini" > "$RALLY_HOME/.rally/rally.ini"
else
  args+=(--kill-running-processes)
fi
[ -n "${TRACK_REPOSITORY:-}" ] && args+=("--track-repository=$TRACK_REPOSITORY")
[ -n "${CHALLENGE:-}" ] && args+=("--challenge=$CHALLENGE")
if [ -n "${INCLUDE_TASKS:-}" ]; then
  args+=("--include-tasks=$INCLUDE_TASKS")
elif [ -n "$EXCLUDE_TASKS" ]; then
  args+=("--exclude-tasks=$EXCLUDE_TASKS")
fi
[ "${TEST_MODE:-0}" = 1 ] && args+=(--test-mode)

mkdir -p "$OUT"
echo "race $NAME -> $TARGET ($DATABASE_TYPE $DIST_VERSION), track $TRACK"
set +e
"$ESRALLY" "${args[@]}" 2>&1 | tee "$OUT/rally-$NAME.log"
status=${PIPESTATUS[0]}
# What the engine answered to each of the track's searches, over the data the race left
# (answers.py), with the Python of the Rally install, which renders the track as Rally does.
RALLY_PYTHON=${RALLY_PYTHON:-$(dirname "$(command -v "$ESRALLY")")/python}
"$RALLY_PYTHON" "$(dirname "$0")/answers.py" "${RALLY_HOME:-$HOME}/.rally/rally.ini" \
  "$TRACK" "$TRACK_PARAMS" "$TARGET" "$OUT/answers-$NAME.json" 2>&1 | tee -a "$OUT/rally-$NAME.log"
set -e
echo "race $NAME exit $status"
exit "$status"
