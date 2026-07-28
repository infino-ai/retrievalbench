#!/usr/bin/env bash
# Runs on the ephemeral VM (piped in via `ssh … bash -s`). Installs the harness,
# installs the infino binding (branch-built wheel scp'd to /tmp/infino_wheel, or
# the published PyPI wheel), then runs one bench leg. Inputs arrive as env vars
# set on the ssh command line: VDB_REPO VDB_REF BINDING INFINO_ENV NUM_PER_BATCH
# CACHE_BUDGET_BYTES BENCH VECTOR_CASE FTS_CASE FTS_DATASET
# PAYLOAD_PROFILE VECTOR_K FTS_K NUM_CONC N_CENT NPROBE RERANK_MULT.
# NUM_PER_BATCH is consumed by the harness; the rest are used below.
set -euo pipefail

# The superfile build opens many fds at scale; the default soft limit (1024)
# trips "Too many open files". Raise the soft limit to the hard limit.
ulimit -n "$(ulimit -Hn)" 2>/dev/null || true

echo "===== APT ====="
sudo DEBIAN_FRONTEND=noninteractive apt-get update -y
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y \
  git wget curl ca-certificates python3 python3-venv python3-dev

echo "===== CLONE HARNESS ====="
rm -rf "$HOME/VectorDBBench"
git clone --depth 1 --branch "$VDB_REF" "https://github.com/${VDB_REPO}.git" "$HOME/VectorDBBench"

echo "===== VENV + HARNESS INSTALL ====="
cd "$HOME/VectorDBBench"
python3 -m venv .venv
source .venv/bin/activate
pip install --upgrade pip
pip install -e '.[infino]'

if [ "$BINDING" = "branch" ]; then
  echo "===== INSTALL BRANCH-BUILT INFINO WHEEL ====="
  pip install --force-reinstall /tmp/infino_wheel/*.whl
else
  echo "===== USING PUBLISHED INFINO WHEEL ====="
fi
python3 -c "import infino; print('infino binding:', getattr(infino, '__version__', 'unknown'))"

echo "===== RUN ($BENCH) ====="
if [ -n "$INFINO_ENV" ]; then
  # Space-separated INFINO_...=value tuning knobs; word-split into multiple
  # exports is intentional.
  # shellcheck disable=SC2086,SC2163
  export $INFINO_ENV
  echo "infino env:"; env | grep '^INFINO_' | sort
fi
export RESULTS_LOCAL_DIR=/tmp/vdb_results
mkdir -p "$RESULTS_LOCAL_DIR"

# Without these the viewer lists the run as an opaque "res-<uuid>" and the
# series as a bare "Infino", while every peer backend carries its hardware.
TASK_LABEL="infino_${BENCH}_$(date -u +%Y%m%d)"
DB_LABEL="${VM_SIZE:-unspecified}"
echo "task label: $TASK_LABEL | db label: $DB_LABEL"

if [ "$BENCH" = "vector" ]; then
  echo "----- VECTOR: $VECTOR_CASE n_cent=$N_CENT nprobe=$NPROBE rerank_mult=$RERANK_MULT -----"
  vectordbbench infino \
    --case-type "$VECTOR_CASE" \
    --k "$VECTOR_K" \
    --num-concurrency "$NUM_CONC" \
    --n-cent "$N_CENT" \
    --nprobe "$NPROBE" \
    --rerank-mult "$RERANK_MULT" \
    --cache-budget-bytes "$CACHE_BUDGET_BYTES" \
    --task-label "$TASK_LABEL" \
    --db-label "$DB_LABEL" \
    --drop-old
else
  echo "----- FTS: $FTS_CASE / $FTS_DATASET -----"
  vectordbbench infinofts \
    --case-type "$FTS_CASE" \
    --dataset-with-size-type "$FTS_DATASET" \
    --payload-profile "$PAYLOAD_PROFILE" \
    --k "$FTS_K" \
    --num-concurrency "$NUM_CONC" \
    --cache-budget-bytes "$CACHE_BUDGET_BYTES" \
    --task-label "$TASK_LABEL" \
    --db-label "$DB_LABEL" \
    --drop-old
fi

echo "===== RESULTS ====="
find "$RESULTS_LOCAL_DIR" -name 'result_*.json' -print
