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
# Engine tuning must go through YAML: infino reads config exclusively from
# files and IGNORES environment variables (its `env_vars_do_not_override_config`
# test pins that). Exporting INFINO_* was therefore a silent no-op — every
# tuning knob passed this way was discarded. Translate the same
# `INFINO_<SECTION>__<KEY>=value` pairs into the user config file that the
# engine does load: $XDG_CONFIG_HOME/infino/config.yaml, falling back to
# $HOME/.config/infino/config.yaml.
if [ -n "$INFINO_ENV" ]; then
  INFINO_CFG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/infino"
  mkdir -p "$INFINO_CFG_DIR"
  INFINO_ENV="$INFINO_ENV" python3 - "$INFINO_CFG_DIR/config.yaml" <<'PYCFG'
import os, sys, collections

# Valid top-level sections of infino's config.yaml. A pair naming anything
# else is a typo we must fail on, not silently drop (the whole point of this
# change is that discarded tuning is invisible).
SECTIONS = {"supertable", "storage", "compaction", "vector", "diagnostics", "memory"}

tree = collections.defaultdict(dict)
for pair in os.environ["INFINO_ENV"].split():
    if "=" not in pair:
        sys.exit(f"INFINO_ENV: expected KEY=value, got {pair!r}")
    key, value = pair.split("=", 1)
    if not key.startswith("INFINO_") or "__" not in key:
        sys.exit(f"INFINO_ENV: expected INFINO_<SECTION>__<KEY>=value, got {key!r}")
    section, _, field = key[len("INFINO_"):].partition("__")
    section, field = section.lower(), field.lower()
    if section not in SECTIONS:
        sys.exit(f"INFINO_ENV: unknown config section {section!r} in {key!r}; "
                 f"valid sections: {sorted(SECTIONS)}")
    tree[section][field] = value

with open(sys.argv[1], "w") as fh:
    for section in sorted(tree):
        fh.write(f"{section}:\n")
        for field, value in sorted(tree[section].items()):
            # Values are emitted bare so ints/floats/bools/enums parse as
            # themselves rather than as strings.
            fh.write(f"  {field}: {value}\n")
PYCFG
  echo "infino config ($INFINO_CFG_DIR/config.yaml):"; cat "$INFINO_CFG_DIR/config.yaml"
fi
export RESULTS_LOCAL_DIR=/tmp/vdb_results
mkdir -p "$RESULTS_LOCAL_DIR"

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
    --drop-old
fi

echo "===== RESULTS ====="
find "$RESULTS_LOCAL_DIR" -name 'result_*.json' -print
