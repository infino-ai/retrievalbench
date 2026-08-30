#!/usr/bin/env bash
# Track A: one pinned binary, one host, all declared corpora/scales.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
TARGET="$ROOT/target"
CORPUS_ROOT="${INFINO_BENCH_CORPUS_DIR:-$TARGET/corpora}"
THREADS="$(getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.logicalcpu)"
ALLOW_DIRTY="${INFINO_BENCH_ALLOW_DIRTY:-0}"

if [ "$(uname -s)" != "Linux" ] && [ "$ALLOW_DIRTY" != "1" ]; then
  echo "official Track A publication requires Linux: PeakSampler uses /proc RSS" >&2
  echo "set INFINO_BENCH_ALLOW_DIRTY=1 only for a non-published smoke run" >&2
  exit 2
fi

export CARGO_TARGET_DIR="$TARGET"
"$ROOT/scripts/build_faiss.sh"
source "$TARGET/faiss-env.sh"

run_one() {
  local run_id="$1"
  local docs="$2"
  local corpus="$3"
  local corpus_dir="$4"
  local command="./scripts/run_track_a.sh $run_id $docs $corpus $corpus_dir"
  local -a corpus_args=("corpus=$corpus" "corpus-dir=$corpus_dir")

  export INFINO_BENCH_SUPERFILE_DOCS="$docs"
  export INFINO_BENCH_SUPERTABLE_DOCS="$docs"
  rm -f "$TARGET/infino-bench"/*.json "$TARGET/infino-bench/host.txt"

  echo "[track-a] run=$run_id docs=$docs corpus=$corpus threads=$THREADS"

  # The codec cells compare RAM-resident quantized indexes, so infino's
  # arm serves the shipped resident-plane mode: one YAML line, applied
  # through the standard config discovery (XDG_CONFIG_HOME), scoped to
  # these two invocations. Every other cell runs shipped defaults.
  FLAT_CFG_HOME="$(mktemp -d)"
  mkdir -p "$FLAT_CFG_HOME/infino"
  printf 'vector:\n  search_mode: flat_ivf\n' > "$FLAT_CFG_HOME/infino/config.yaml"

  INFINO_BENCH_THREAD_MODE=single-thread \
    XDG_CONFIG_HOME="$FLAT_CFG_HOME" \
    RAYON_NUM_THREADS=1 OMP_NUM_THREADS=1 \
    cargo bench --features faiss --bench bench -- \
      track-a-codec "${corpus_args[@]}"

  INFINO_BENCH_THREAD_MODE=box-threads \
    XDG_CONFIG_HOME="$FLAT_CFG_HOME" \
    RAYON_NUM_THREADS="$THREADS" OMP_NUM_THREADS="$THREADS" \
    cargo bench --features faiss --bench bench -- \
      track-a-codec "${corpus_args[@]}"

  rm -rf "$FLAT_CFG_HOME"

  RAYON_NUM_THREADS="$THREADS" OMP_NUM_THREADS="$THREADS" \
    cargo bench --features faiss --bench bench -- \
      track-a-writes "${corpus_args[@]}"

  RAYON_NUM_THREADS="$THREADS" OMP_NUM_THREADS="$THREADS" \
    cargo bench --features faiss --bench bench -- \
      superfile vector "${corpus_args[@]}"

  RAYON_NUM_THREADS="$THREADS" OMP_NUM_THREADS="$THREADS" \
    cargo bench --features faiss --bench bench -- \
      supertable vector build warm cold "${corpus_args[@]}"

  local -a publish_args=(
    --src "$TARGET/infino-bench"
    --dst "$ROOT/results/inprocess"
    --run-id "$run_id"
    --corpus "$corpus"
    --docs "$docs"
    --command "$command"
  )
  if [ "$ALLOW_DIRTY" = "1" ]; then
    publish_args+=(--allow-dirty)
  fi
  python3 "$ROOT/scripts/publish_inprocess.py" "${publish_args[@]}"
}

if [ "$#" -eq 0 ]; then
  mkdir -p "$CORPUS_ROOT"
  run_one dbpedia-1536-100k 100000 \
    hf:KShivendu/dbpedia-entities-openai-1M "$CORPUS_ROOT"
  run_one glove-200-100k 100000 \
    annb:glove-200-angular "$CORPUS_ROOT"
  run_one glove-200-1m 1000000 \
    annb:glove-200-angular "$CORPUS_ROOT"
  for docs in 100000 1000000 10000000; do
    cohere_dir="$CORPUS_ROOT/cohere-$docs"
    "$ROOT/scripts/prepare_cohere.sh" "$docs" "$cohere_dir"
    run_one "cohere-768-${docs}" "$docs" "parquet:$cohere_dir" "$CORPUS_ROOT"
  done
elif [ "$#" -eq 4 ]; then
  run_one "$1" "$2" "$3" "$4"
else
  echo "usage: $0 [run-id docs corpus corpus-dir]" >&2
  exit 2
fi
