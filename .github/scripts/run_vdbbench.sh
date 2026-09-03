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

# Label runs with the viewer's default-selected task labels so our numbers land
# on the leaderboard pages without hand-selecting a filter: the qps/recall page
# defaults to "standard_20260403", and the full-text page groups the bundled set
# under "fts_standard". These track the viewer's current defaults — bump them if
# the viewer moves its default to a newer standard_<date>. DB_LABEL still carries
# the hardware (VM size) so our series is distinguishable from the peer backends,
# which ran on their own reference hardware.
if [ "$BENCH" = "vector" ]; then
  TASK_LABEL="standard_20260403"
else
  TASK_LABEL="fts_standard"
fi
DB_LABEL="${DB_LABEL:-${VM_SIZE:-unspecified}}"
echo "task label: $TASK_LABEL | db label: $DB_LABEL"

if [ "$BENCH" = "vector" ]; then
  echo "----- VECTOR: $VECTOR_CASE n_cent=$N_CENT nprobe=$NPROBE rerank_mult=$RERANK_MULT" \
       "search_mode=${SEARCH_MODE:-client-default} ef_sweep=${EF_SWEEP:-none} -----"
  # Search/index knobs ride only when explicitly set: unset means the
  # engine's manifest-calibrated defaults (the recall-validated path).
  TUNING=()
  [ -n "$N_CENT" ] && TUNING+=(--n-cent "$N_CENT")
  [ -n "$NPROBE" ] && TUNING+=(--nprobe "$NPROBE")
  [ -n "$RERANK_MULT" ] && TUNING+=(--rerank-mult "$RERANK_MULT")
  [ -n "${SEARCH_MODE:-}" ] && TUNING+=(--search-mode "$SEARCH_MODE")
  # ef sweep: one bench run per serve-time beam. The beam varies via the client's
  # --ef (engine vector.hnsw_ef_search), a SERVE-time knob — so BUILD ONCE: the
  # first beam ingests+builds (--drop-old); every later beam reuses that build and
  # only re-searches (--skip-drop-old --skip-load). That's the whole point of the
  # serve-time ef (vs a build-time target_recall that would re-ingest per point).
  # Unset EF_SWEEP = one run at the stamped k->ef curve.
  #
  # Each beam gets its OWN task label ("${TASK_LABEL}_ef<v>"): VDBBench names a
  # result file by date+task_label+db, so a shared label makes every beam
  # overwrite the previous one — the whole sweep would collapse to a single
  # point. The per-beam files are stitched back into one file under $TASK_LABEL
  # after the loop, so the viewer plots a single Infino series with one point per
  # ef (its qps/recall page groups points by db_name and reads every entry in the
  # file). db-label stays constant so all points land on the same series.
  EF_VALUES=(${EF_SWEEP:-})
  [ ${#EF_VALUES[@]} -eq 0 ] && EF_VALUES=("")
  first=1
  for EFV in "${EF_VALUES[@]}"; do
    EF_FLAG=()
    [ -n "$EFV" ] && EF_FLAG=(--ef "$EFV")
    if [ "$first" = "1" ]; then
      LOAD_FLAGS=(--drop-old)
      first=0
    else
      LOAD_FLAGS=(--skip-drop-old --skip-load)
    fi
    LEG_LABEL="${TASK_LABEL}_ef${EFV:-curve}"
    echo "----- vector run: ef=${EFV:-stamped-curve} (${LOAD_FLAGS[*]}) label=$LEG_LABEL -----"
    vectordbbench infino \
      --case-type "$VECTOR_CASE" \
      --k "$VECTOR_K" \
      --num-concurrency "$NUM_CONC" \
      "${TUNING[@]}" \
      "${EF_FLAG[@]}" \
      --cache-budget-bytes "$CACHE_BUDGET_BYTES" \
      --task-label "$LEG_LABEL" \
      --db-label "$DB_LABEL" \
      "${LOAD_FLAGS[@]}"
  done
  # Stitch the per-beam result files into single-curve files. We emit the same
  # curve under TWO labels: $TASK_LABEL (the viewer's default-selected set, so it
  # shows on qps/recall without hand-selecting a filter) and infino_vector_<date>
  # (a clean per-run snapshot, filterable to just infino). Same db_name in both,
  # so each renders as one Infino series with a point per ef.
  python3 - "$RESULTS_LOCAL_DIR" "$TASK_LABEL" "$VECTOR_CASE" <<'PYMERGE'
import glob, json, os, re, sys

results_dir, base_label, vector_case = sys.argv[1], sys.argv[2], sys.argv[3]
# Case token keeps 1M and 10M in distinct files/run_ids under the same task
# label — the bucket key is date+label+case, not just date+label, so a
# same-day 1M run no longer overwrites the 10M curve (or vice versa).
case_tok = re.sub(r"[^a-z0-9]", "", vector_case.lower())
infino_dir = os.path.join(results_dir, "Infino")
legs = sorted(glob.glob(os.path.join(infino_dir, f"result_*_{base_label}_ef*_infino.json")))
if not legs:
    sys.exit(f"no per-beam result files to merge for {base_label!r}")
merged, entries = None, []
for fp in legs:
    with open(fp) as fh:
        doc = json.load(fh)
    if merged is None:
        merged = doc
    entries.extend(doc.get("results", []))
date_m = re.search(r"result_(\d{8})_", os.path.basename(legs[0]))
run_date = date_m.group(1) if date_m else "unknown"
written = []
for label in (base_label, f"infino_vector_{run_date}"):
    doc = dict(merged)
    doc["task_label"] = label
    # Distinct run_id per file: the viewer groups result files by run_id and
    # MERGES any that share one (keeping only the first file's task_label), so a
    # shared run_id would collapse these two into one label and hide the other.
    doc["run_id"] = f"{label}_{case_tok}_{run_date}"
    doc["results"] = entries
    out = os.path.join(infino_dir, f"result_{run_date}_{label}_{case_tok}_infino.json")
    with open(out, "w") as fh:
        json.dump(doc, fh, indent=2)
    written.append(f"{os.path.basename(out)} [run_id={doc['run_id']} task_label={doc['task_label']} n={len(doc['results'])}]")
for fp in legs:
    os.remove(fp)
print(f"stitched {len(legs)} beam(s) ({len(entries)} point(s)):")
for w in written:
    print(f"  -> {w}")
PYMERGE
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
