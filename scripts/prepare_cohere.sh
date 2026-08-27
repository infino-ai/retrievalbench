#!/usr/bin/env bash
# Stage the public VectorDBBench Cohere corpus as Infino parquet input.
set -euo pipefail

DOCS="${1:?usage: prepare_cohere.sh <100000|1000000|10000000> <directory>}"
DEST="${2:?usage: prepare_cohere.sh <100000|1000000|10000000> <directory>}"
BASE="https://assets.zilliz.com/benchmark"

case "$DOCS" in
  100000)
    DATASET="cohere_small_100k"
    SHARDS=("train.parquet")
    ;;
  1000000)
    DATASET="cohere_medium_1m"
    SHARDS=("train.parquet")
    ;;
  10000000)
    DATASET="cohere_large_10m"
    SHARDS=()
    for shard in $(seq -w 0 9); do
      SHARDS+=("train-${shard}-of-10.parquet")
    done
    ;;
  *)
    echo "unsupported Cohere size: $DOCS" >&2
    exit 2
    ;;
esac

mkdir -p "$DEST"
for shard in "${SHARDS[@]}"; do
  curl --fail --location --retry 5 --continue-at - \
    --output "$DEST/$shard" "$BASE/$DATASET/$shard"
done
# Keep held-out queries after every train shard in lexical file order.
curl --fail --location --retry 5 --continue-at - \
  --output "$DEST/zz-test.parquet" "$BASE/$DATASET/test.parquet"

mkdir -p "$DEST/ground-truth"
case "$DOCS" in
  100000)
    GT_DATASETS=("100000:cohere_small_100k")
    ;;
  1000000)
    GT_DATASETS=("1000000:cohere_medium_1m")
    ;;
  10000000)
    GT_DATASETS=(
      "100000:cohere_small_100k"
      "1000000:cohere_medium_1m"
      "10000000:cohere_large_10m"
    )
    ;;
esac
for item in "${GT_DATASETS[@]}"; do
  size="${item%%:*}"
  dataset="${item#*:}"
  curl --fail --location --retry 5 --continue-at - \
    --output "$DEST/ground-truth/$size.parquet" \
    "$BASE/$dataset/neighbors.parquet"
done

echo "$DEST"
