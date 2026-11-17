#!/usr/bin/env bash
# The Rally head-to-head on Google Cloud: one VM per engine and one Rally runner, the same
# shape as the AWS setup (see README.md), with gcloud in place of Terraform and the
# engines run from their release tarballs.
#
#   1. create the VMs (labelled, with a per-run SSH key and per-run firewall rules)
#   2. install and start each engine (engine-setup.sh) and Rally (runner-setup.sh)
#   3. race each engine from the runner (race.sh), sampling the engine VM's CPU and memory
#      (sampler.py), and measure each engine's data directory when its race ends
#   4. copy everything to opensearch-rally/results/<run>/ and write the comparison (compare.py)
#   5. delete the VMs and firewall rules, unless KEEP=1
#
# Usage: opensearch-rally/benchmark.sh   (configured by the environment; see README.md)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

PROJECT=${PROJECT:?set PROJECT to the GCP project the VMs go in}
ZONE=${ZONE:-us-east4-a}
REGION=${ZONE%-*}
NETWORK=${NETWORK:-default}
SUBNET=${SUBNET:-default}
VM_SA=${VM_SA:-}
ENGINES=${ENGINES:-elasticsearch,opensearch,opensearch-infino}
# n2-highmem-4 (4 vCPU, 32 GB) is the i3en.xlarge the AWS setup ran engines and the runner
# on; each local SSD is 375 GB of NVMe.
ENGINE_MACHINE=${ENGINE_MACHINE:-n2-highmem-4}
RUNNER_MACHINE=${RUNNER_MACHINE:-n2-highmem-4}
ENGINE_SSDS=${ENGINE_SSDS:-1}
RUNNER_SSDS=${RUNNER_SSDS:-1}
IMAGE_FAMILY=${IMAGE_FAMILY:-ubuntu-2404-lts-amd64}
ES_VERSION=${ES_VERSION:-9.1.3}
OS_VERSION=${OS_VERSION:-3.9.0}
# Rally's --distribution-version: the AWS setup passed the Elasticsearch version for every
# engine, which picks the track revision all engines run.
RALLY_DISTRIBUTION_VERSION=${RALLY_DISTRIBUTION_VERSION:-$ES_VERSION}
HEAP_GB=${HEAP_GB:-16}

# The AWS setup's workload knobs and defaults; unset knobs keep the track's own default.
TRACK=${TRACK:-nested}
CHALLENGE=${CHALLENGE:-}
TRACK_REPOSITORY=${TRACK_REPOSITORY:-}
CORPUS_SIZE=${CORPUS_SIZE:-}
INGEST_PERCENTAGE=${INGEST_PERCENTAGE:-}
BULK_INDEXING_CLIENTS=${BULK_INDEXING_CLIENTS:-8}
BULK_SIZE=${BULK_SIZE:-}
TARGET_THROUGHPUT=${TARGET_THROUGHPUT:-}
EXCLUDE_TASKS=${EXCLUDE_TASKS:-indices-stats}
# Tasks only the Infino engines skip, as the AWS setup skipped inner-hits queries for its
# Infino arm; a skipped task is left out of every engine's comparison.
INFINO_EXCLUDE_TASKS=${INFINO_EXCLUDE_TASKS:-}
INCLUDE_TASKS=${INCLUDE_TASKS:-}
TEST_MODE=${TEST_MODE:-0}
# Sequential races leave each engine the runner's whole CPU and network; PARALLEL=1 races all
# engines at once, as the AWS setup did.
PARALLEL=${PARALLEL:-0}

PLUGIN_ZIP=${PLUGIN_ZIP:-}
BRIDGE_LIB=${BRIDGE_LIB:-}
INFINO_YAML=${INFINO_YAML:-}
INFINO_PLATFORM_URL=${INFINO_PLATFORM_URL:-}
INFINO_DATABASE=${INFINO_DATABASE:-}
INFINO_API_KEY=${INFINO_API_KEY:-}
INFINO_MODES=${INFINO_MODES:-1}

KEEP=${KEEP:-0}
TTL_HOURS=${TTL_HOURS:-12}
RESULTS_BUCKET=${RESULTS_BUCKET:-}
# The hourly price of one ENGINE_MACHINE with its SSDs; when set, the report adds each
# engine's cost of the race.
ENGINE_PRICE_PER_HOUR=${ENGINE_PRICE_PER_HOUR:-}

ADMIN=bench
SAMPLE_INTERVAL_SECS=5
# How long to wait for a new VM to take SSH: tries, and seconds between them.
SSH_TRIES=60
SSH_WAIT_SECS=5

IFS=',' read -r -a ENGINE_LIST <<< "$ENGINES"
for e in "${ENGINE_LIST[@]}"; do
  case "$e" in
    elasticsearch|opensearch) ;;
    opensearch-infino|opensearch-infino-proxy)
      : "${PLUGIN_ZIP:?$e needs PLUGIN_ZIP (plugin/build/distributions/infino-engine.zip)}"
      : "${BRIDGE_LIB:?$e needs BRIDGE_LIB (libinfino_capi.so)}" ;;
    *) echo "unknown engine: $e" >&2; exit 1 ;;
  esac
  if [ "$e" = opensearch-infino-proxy ]; then
    : "${INFINO_PLATFORM_URL:?proxy mode needs INFINO_PLATFORM_URL}"
    : "${INFINO_API_KEY:?proxy mode needs INFINO_API_KEY}"
  fi
done

STAMP=$(date -u +%Y%m%d-%H%M%S)
RUN=${RUN:-rally-$STAMP}
OUT=$HERE/results/$RUN
mkdir -p "$OUT"
KEY=$OUT/ssh-key
ssh-keygen -t ed25519 -N '' -q -f "$KEY"
SSH_OPTS=(-i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR
  -o ServerAliveInterval=15 -o ServerAliveCountMax=4)
DELETE_AFTER=$(date -u -d "+${TTL_HOURS} hours" +%s)
LABELS="purpose=rally-bench,run-id=$RUN,auto-delete=true,delete-after=$DELETE_AFTER"
RUNNER_VM=$RUN-runner
ENGINE_TAG=$RUN-engine
VMS=("$RUNNER_VM")
for e in "${ENGINE_LIST[@]}"; do VMS+=("$RUN-$e"); done

gc() { gcloud --project "$PROJECT" --quiet "$@"; }
on() { local vm=$1; shift; ssh "${SSH_OPTS[@]}" "$ADMIN@${EXT_IP[$vm]}" "$@"; }
put() { local vm=$1; shift; scp "${SSH_OPTS[@]}" -q "$@" "$ADMIN@${EXT_IP[$vm]}:bench/"; }
get() { scp "${SSH_OPTS[@]}" -q "$ADMIN@${EXT_IP[$1]}:$2" "$3"; }

teardown() {
  local status=$?
  if [ "$KEEP" = 1 ]; then
    echo "KEEP=1: leaving ${VMS[*]} running; delete them with"
    echo "  gcloud --project $PROJECT compute instances delete --zone $ZONE ${VMS[*]}"
    echo "  gcloud --project $PROJECT compute firewall-rules delete $RUN-ssh $RUN-int"
  else
    echo "deleting ${VMS[*]}"
    gc compute instances delete --zone "$ZONE" "${VMS[@]}" >/dev/null 2>&1 || true
    gc compute firewall-rules delete "$RUN-ssh" "$RUN-int" >/dev/null 2>&1 || true
  fi
  exit "$status"
}
trap teardown EXIT
trap 'exit 130' INT TERM

echo "run $RUN in $PROJECT/$ZONE: engines ${ENGINE_LIST[*]}, track $TRACK"

# 1. VMs and firewall rules.
# The VMs call no Google API, so by default they run with no service account.
SA_ARGS=(--no-service-account --no-scopes)
[ -n "$VM_SA" ] && SA_ARGS=(--service-account "$VM_SA" --scopes storage-rw)
create_vm() {
  local vm=$1 machine=$2 ssds=$3 tag=$4 ssd_args=()
  for _ in $(seq 1 "$ssds"); do ssd_args+=(--local-ssd=interface=NVME); done
  gc compute instances create "$vm" --zone "$ZONE" \
    --machine-type "$machine" "${ssd_args[@]}" \
    --image-family "$IMAGE_FAMILY" --image-project ubuntu-os-cloud \
    --boot-disk-size 64GB --subnet "$SUBNET" "${SA_ARGS[@]}" \
    --tags "$RUN,$tag" --labels "$LABELS" \
    --description "rally head-to-head $RUN" \
    --metadata "enable-oslogin=FALSE,ssh-keys=$ADMIN:$(< "$KEY.pub")" >/dev/null
}
create_vm "$RUNNER_VM" "$RUNNER_MACHINE" "$RUNNER_SSDS" "$RUN-runner" &
for e in "${ENGINE_LIST[@]}"; do create_vm "$RUN-$e" "$ENGINE_MACHINE" "$ENGINE_SSDS" "$ENGINE_TAG" & done
wait

OPERATOR_IP=$(curl -fsS https://api.ipify.org)
gc compute firewall-rules create "$RUN-ssh" --network "$NETWORK" --direction INGRESS \
  --allow tcp:22 --source-ranges "$OPERATOR_IP/32" --target-tags "$RUN" >/dev/null
gc compute firewall-rules create "$RUN-int" --network "$NETWORK" --direction INGRESS \
  --allow tcp:9200 --source-tags "$RUN-runner" --target-tags "$ENGINE_TAG" >/dev/null

declare -A EXT_IP INT_IP
for vm in "${VMS[@]}"; do
  EXT_IP[$vm]=$(gc compute instances describe "$vm" --zone "$ZONE" \
    --format 'get(networkInterfaces[0].accessConfigs[0].natIP)')
  INT_IP[$vm]=$(gc compute instances describe "$vm" --zone "$ZONE" \
    --format 'get(networkInterfaces[0].networkIP)')
done
for vm in "${VMS[@]}"; do
  for i in $(seq 1 $SSH_TRIES); do
    ssh "${SSH_OPTS[@]}" -o ConnectTimeout=5 "$ADMIN@${EXT_IP[$vm]}" true 2>/dev/null && break
    [ "$i" = $SSH_TRIES ] && { echo "$vm never took SSH" >&2; exit 1; }
    sleep $SSH_WAIT_SECS
  done
  on "$vm" mkdir -p bench
done
echo "VMs up: $(for vm in "${VMS[@]}"; do printf '%s=%s ' "$vm" "${EXT_IP[$vm]}"; done)"

# 2. Engines and Rally.
put "$RUNNER_VM" "$HERE/runner-setup.sh" "$HERE/race.sh"
on "$RUNNER_VM" bash bench/runner-setup.sh > "$OUT/setup-runner.log" 2>&1 &
pids=($!)
for e in "${ENGINE_LIST[@]}"; do
  vm=$RUN-$e
  files=("$HERE/engine-setup.sh" "$HERE/sampler.py")
  case "$e" in
    opensearch-infino*)
      cp "$PLUGIN_ZIP" "$OUT/infino-engine.zip"
      cp "$BRIDGE_LIB" "$OUT/libinfino_capi.so"
      files+=("$OUT/infino-engine.zip" "$OUT/libinfino_capi.so")
      [ -n "$INFINO_YAML" ] && { cp "$INFINO_YAML" "$OUT/infino.yaml"; files+=("$OUT/infino.yaml"); } ;;
  esac
  put "$vm" "${files[@]}"
  version=$OS_VERSION
  [ "$e" = elasticsearch ] && version=$ES_VERSION
  # The account settings go over the SSH session's stdin, so the API key is never on a
  # command line.
  printf 'export INFINO_PLATFORM_URL=%q INFINO_DATABASE=%q INFINO_API_KEY=%q INFINO_MODES=%q\nbash bench/engine-setup.sh %q %q %q\n' \
    "$INFINO_PLATFORM_URL" "$INFINO_DATABASE" "$INFINO_API_KEY" "$INFINO_MODES" "$e" "$version" "$HEAP_GB" \
    | on "$vm" bash -s > "$OUT/setup-$e.log" 2>&1 &
  pids+=($!)
done
for p in "${pids[@]}"; do
  wait "$p" || { echo "setup failed; see $OUT/setup-*.log" >&2; exit 1; }
done
rm -f "$OUT/infino-engine.zip" "$OUT/libinfino_capi.so"
for e in "${ENGINE_LIST[@]}"; do
  on "$RUNNER_VM" curl -fsS "http://${INT_IP[$RUN-$e]}:9200" >/dev/null \
    || { echo "runner cannot reach $e" >&2; exit 1; }
done

# 3. Races.
params="{\"number_of_replicas\":0"
[ -n "$CORPUS_SIZE" ] && params+=",\"corpus_size\":$CORPUS_SIZE"
[ -n "$INGEST_PERCENTAGE" ] && params+=",\"ingest_percentage\":$INGEST_PERCENTAGE"
[ -n "$BULK_INDEXING_CLIENTS" ] && params+=",\"bulk_indexing_clients\":$BULK_INDEXING_CLIENTS"
[ -n "$BULK_SIZE" ] && params+=",\"bulk_size\":$BULK_SIZE"
[ -n "$TARGET_THROUGHPUT" ] && params+=",\"target_throughput\":\"$TARGET_THROUGHPUT\""
params+="}"

race_one() {
  local e=$1 vm=$RUN-$1 db=opensearch exclude=$EXCLUDE_TASKS
  [ "$e" = elasticsearch ] && db=elasticsearch
  case "$e" in
    opensearch-infino*) [ -n "$INFINO_EXCLUDE_TASKS" ] && exclude="$exclude,$INFINO_EXCLUDE_TASKS" ;;
  esac
  on "$vm" "rm -f /mnt/data/sampler.stop; nohup python3 bench/sampler.py /mnt/data/engine.pid \
    /mnt/data/metrics.jsonl /mnt/data/sampler.stop $SAMPLE_INTERVAL_SECS </dev/null >/dev/null 2>&1 &"
  local status=0
  printf 'export ESRALLY=/mnt/rally/venv/bin/esrally TRACK=%q TRACK_REPOSITORY=%q CHALLENGE=%q TRACK_PARAMS=%q EXCLUDE_TASKS=%q INCLUDE_TASKS=%q TEST_MODE=%q PARALLEL=%q\nbash bench/race.sh %q %q %q %q /mnt/rally/results\n' \
    "$TRACK" "$TRACK_REPOSITORY" "$CHALLENGE" "$params" "$exclude" "$INCLUDE_TASKS" "$TEST_MODE" "$PARALLEL" \
    "$e" "${INT_IP[$vm]}:9200" "$db" "$RALLY_DISTRIBUTION_VERSION" \
    | on "$RUNNER_VM" bash -s 2>&1 | sed -u "s/^/[$e] /" || status=$?
  # The sampler writes its last line within one interval of the stop file.
  on "$vm" "touch /mnt/data/sampler.stop; sleep $((SAMPLE_INTERVAL_SECS + 1))"
  # The whole data directory: the Lucene indices, and for the Infino engines the table too.
  on "$vm" "du -sb /mnt/data/data /mnt/data/infino-catalog 2>/dev/null | awk '{s+=\$1} END {print s}'" \
    > "$OUT/storage-$e.txt"
  on "$vm" "curl -s 'localhost:9200/_cat/indices?bytes=b&h=index,docs.count,store.size'" > "$OUT/indices-$e.txt" || true
  get "$vm" /mnt/data/metrics.jsonl "$OUT/metrics-$e.jsonl" || true
  get "$vm" /mnt/data/machine.json "$OUT/machine-$e.json" || true
  get "$RUNNER_VM" "/mnt/rally/results/rally-$e.csv" "$OUT/rally-$e.csv" || echo "[$e] no Rally report"
  get "$RUNNER_VM" "/mnt/rally/results/rally-$e.log" "$OUT/rally-$e.log" || true
  echo "[$e] race exit $status"
}

if [ "$PARALLEL" = 1 ]; then
  for e in "${ENGINE_LIST[@]}"; do race_one "$e" & done
  wait
else
  for e in "${ENGINE_LIST[@]}"; do race_one "$e"; done
fi

# 4. Report.
{
  echo "run=$RUN project=$PROJECT zone=$ZONE"
  echo "engine_machine=$ENGINE_MACHINE x$ENGINE_SSDS local SSD, runner_machine=$RUNNER_MACHINE x$RUNNER_SSDS local SSD"
  echo "es=$ES_VERSION opensearch=$OS_VERSION heap=${HEAP_GB}g rally_distribution_version=$RALLY_DISTRIBUTION_VERSION"
  echo "track=$TRACK challenge=${CHALLENGE:-default} track_params=$params parallel=$PARALLEL test_mode=$TEST_MODE"
  echo "exclude_tasks=$EXCLUDE_TASKS infino_exclude_tasks=$INFINO_EXCLUDE_TASKS include_tasks=$INCLUDE_TASKS"
} > "$OUT/run.txt"
[ "$KEEP" = 1 ] || rm -f "$KEY" "$KEY.pub"
PRICE_ARGS=()
[ -n "$ENGINE_PRICE_PER_HOUR" ] && PRICE_ARGS=("--price-per-hour=$ENGINE_PRICE_PER_HOUR")
python3 "$HERE/compare.py" "${PRICE_ARGS[@]}" "$OUT" "${ENGINE_LIST[@]}"
if [ -n "$RESULTS_BUCKET" ]; then
  gcloud storage cp -r "$OUT" "$RESULTS_BUCKET/rally-results/" >/dev/null
  echo "uploaded to $RESULTS_BUCKET/rally-results/$RUN"
fi
echo "results in $OUT"
