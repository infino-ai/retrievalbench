#!/bin/bash
# Reproducible ann-benchmarks recall run for infino.
#   ./run.sh <infino-catalog-dir> <hdf5-data-dir> [dataset ...]
# Uses $PYTHON (a venv with the published `infino` wheel + h5py/pyarrow/numpy).
set -u
CONN=${1:?usage: run.sh <infino-catalog-dir> <hdf5-data-dir> [dataset ...]}
DATA=${2:?usage: run.sh <infino-catalog-dir> <hdf5-data-dir> [dataset ...]}
shift 2
PY=${PYTHON:-python3}
# Full infino-supported roster (metric derived from the name suffix); jaccard excluded.
DS=${*:-"sift-128-euclidean gist-960-euclidean mnist-784-euclidean fashion-mnist-784-euclidean \
glove-25-angular glove-50-angular glove-100-angular glove-200-angular \
nytimes-256-angular deep-image-96-angular lastfm-64-dot"}

mkdir -p "$DATA"
# Pre-download the HDF5 via curl — ann-benchmarks.com 403s the default urllib UA.
for ds in $DS; do
  f="$DATA/$ds.hdf5"
  if [ -s "$f" ]; then echo "have $ds"; else
    echo "downloading $ds"; curl -fsSL -C - -o "$f" "https://ann-benchmarks.com/$ds.hdf5" || echo "FAILED $ds"
  fi
done
exec "$PY" "$(dirname "$0")/annbench_infino.py" "$CONN" "$DATA" $DS
