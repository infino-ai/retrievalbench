#!/usr/bin/env bash
# Runs on an engine VM (as the admin user, with sudo): puts the engine's data on the local
# SSD, installs one engine from its release tarball, starts it on :9200 and waits for it.
# benchmark.sh copies this script, and for the Infino engines the plugin zip, the bridge
# library and infino.yaml, into ~/bench first.
#
# Every engine runs from its tarball on the host rather than from a container image: the
# bridge library links against the host's glibc, which the OpenSearch image does not have,
# and running all three the same way keeps the comparison even.
#
# Usage: engine-setup.sh <engine> <version> <heap GB>
#   engine: elasticsearch | opensearch | opensearch-infino | opensearch-infino-proxy
# Environment (opensearch-infino-proxy only): INFINO_PLATFORM_URL, INFINO_DATABASE,
#   INFINO_API_KEY.
set -euo pipefail

ENGINE=$1
VERSION=$2
HEAP_GB=$3

BENCH=$HOME/bench
DATA=/mnt/data
# The local SSDs GCE attaches with --local-ssd=interface=NVME.
LOCAL_SSD_GLOB='/dev/disk/by-id/google-local-nvme-ssd-*'
# OpenSearch and Elasticsearch refuse to start below this mmap count.
MAX_MAP_COUNT=262144
# Open files the engine process may hold.
NOFILE=65536
# Seconds between health checks, and how many to try before giving up.
HEALTH_WAIT_SECS=2
HEALTH_TRIES=900

sudo DEBIAN_FRONTEND=noninteractive apt-get update -y >/dev/null
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y curl jq python3 mdadm >/dev/null

# The engine's data, logs and install go on the local NVMe SSDs, as the AWS harness put them
# on the i3en instance store; several SSDs are striped into one volume.
mapfile -t SSDS < <(ls $LOCAL_SSD_GLOB 2>/dev/null || true)
sudo mkdir -p "$DATA"
if [ "${#SSDS[@]}" -gt 1 ]; then
  sudo mdadm --create /dev/md0 --run --level=0 --raid-devices="${#SSDS[@]}" "${SSDS[@]}"
  sudo mkfs.ext4 -F -q /dev/md0
  sudo mount /dev/md0 "$DATA"
elif [ "${#SSDS[@]}" = 1 ]; then
  sudo mkfs.ext4 -F -q "${SSDS[0]}"
  sudo mount "${SSDS[0]}" "$DATA"
else
  echo "warning: no local SSD; using the boot disk" >&2
fi
sudo chown "$(id -u):$(id -g)" "$DATA"
sudo sysctl -q -w vm.max_map_count=$MAX_MAP_COUNT

case "$ENGINE" in
  elasticsearch)
    URL="https://artifacts.elastic.co/downloads/elasticsearch/elasticsearch-${VERSION}-linux-x86_64.tar.gz"
    BIN=elasticsearch ;;
  opensearch|opensearch-infino|opensearch-infino-proxy)
    URL="https://artifacts.opensearch.org/releases/bundle/opensearch/${VERSION}/opensearch-${VERSION}-linux-x64.tar.gz"
    BIN=opensearch ;;
  *) echo "unknown engine: $ENGINE" >&2; exit 1 ;;
esac

mkdir -p "$DATA/dist" "$DATA/engine" "$DATA/data" "$DATA/logs"
curl -fsSL --retry 5 --retry-delay 2 -o "$DATA/dist/engine.tar.gz" "$URL"
tar -xzf "$DATA/dist/engine.tar.gz" -C "$DATA/engine" --strip-components=1
HOME_DIR=$DATA/engine
CONF=$HOME_DIR/config

# The settings the AWS harness's containers ran with: single node, the heap locked in memory.
{
  echo "discovery.type: single-node"
  echo "network.host: 0.0.0.0"
  echo "http.port: 9200"
  echo "path.data: $DATA/data"
  echo "path.logs: $DATA/logs"
  echo "bootstrap.memory_lock: true"
} > "$CONF/$BIN.yml"
mkdir -p "$CONF/jvm.options.d"
printf -- '-Xms%sg\n-Xmx%sg\n' "$HEAP_GB" "$HEAP_GB" > "$CONF/jvm.options.d/heap.options"

if [ "$BIN" = opensearch ]; then
  echo "plugins.security.disabled: true" >> "$CONF/$BIN.yml"
  # The bundle's neural-search plugin and the Infino plugin each register a query-phase
  # searcher, and OpenSearch starts with at most one. It is removed from every OpenSearch
  # engine, stock included, so the OpenSearch engines differ only by the Infino plugin.
  if [ -d "$HOME_DIR/plugins/opensearch-neural-search" ]; then
    "$HOME_DIR/bin/opensearch-plugin" remove opensearch-neural-search
  fi
fi

case "$ENGINE" in
  elasticsearch)
    echo "xpack.security.enabled: false" >> "$CONF/$BIN.yml" ;;
  opensearch-infino|opensearch-infino-proxy)
    "$HOME_DIR/bin/opensearch-plugin" install -b "file://$BENCH/infino-engine.zip"
    mkdir -p "$DATA/infino" "$DATA/infino-catalog"
    cp "$BENCH/libinfino_capi.so" "$DATA/infino/libinfino_capi.so"
    {
      echo "-Dinfino.lib=$DATA/infino/libinfino_capi.so"
      echo "-Dinfino.uri=file://$DATA/infino-catalog"
      echo "-Dinfino.data=$DATA/infino-catalog"
    } > "$CONF/jvm.options.d/infino.options"
    # The engine reads infino.yaml from the node's working directory, which bin/opensearch
    # sets to the OpenSearch home.
    if [ -f "$BENCH/infino.yaml" ]; then cp "$BENCH/infino.yaml" "$HOME_DIR/infino.yaml"; fi
    ;;
esac

if [ "$ENGINE" = opensearch-infino-proxy ]; then
  : "${INFINO_PLATFORM_URL:?proxy mode needs INFINO_PLATFORM_URL}"
  : "${INFINO_API_KEY:?proxy mode needs INFINO_API_KEY}"
  echo "infino.platform.url: $INFINO_PLATFORM_URL" >> "$CONF/$BIN.yml"
  if [ -n "${INFINO_DATABASE:-}" ]; then echo "infino.platform.database: $INFINO_DATABASE" >> "$CONF/$BIN.yml"; fi
  echo "infino.default_mode.enabled: false" >> "$CONF/$BIN.yml"
  [ -f "$CONF/opensearch.keystore" ] || "$HOME_DIR/bin/opensearch-keystore" create
  printf '%s' "$INFINO_API_KEY" | "$HOME_DIR/bin/opensearch-keystore" add --stdin --force infino.platform.api_key
fi

# The limits the AWS harness's containers had (memlock unlimited for bootstrap.memory_lock,
# nofile 65536), raised on this shell so the engine inherits them.
sudo prlimit --pid $$ --memlock=unlimited:unlimited --nofile=$NOFILE:$NOFILE
"$HOME_DIR/bin/$BIN" -d -p "$DATA/engine.pid" </dev/null >/dev/null 2>&1

for i in $(seq 1 $HEALTH_TRIES); do
  health=$(curl -s "http://localhost:9200/_cluster/health?wait_for_status=yellow&timeout=1s" || true)
  case "$health" in *'"status":"green"'*|*'"status":"yellow"'*) break ;; esac
  # A node that failed to start has exited; report it now rather than at the deadline.
  dead=0
  if [ -f "$DATA/engine.pid" ] && ! kill -0 "$(< "$DATA/engine.pid")" 2>/dev/null; then dead=1; fi
  if [ "$dead" = 1 ] || [ "$i" = "$HEALTH_TRIES" ]; then
    [ "$dead" = 1 ] && echo "engine exited during start-up; last log lines:" >&2
    [ "$dead" = 1 ] || echo "engine never became healthy; last log lines:" >&2
    tail -n 50 "$DATA/logs/"*.log >&2 || true
    exit 1
  fi
  sleep $HEALTH_WAIT_SECS
done

# What the engine ran on, for the report.
python3 - "$ENGINE" "$VERSION" > "$DATA/machine.json" <<'PY'
import json, os, subprocess, sys
model = next((l.split(":", 1)[1].strip() for l in open("/proc/cpuinfo") if l.startswith("model name")), "unknown")
mem_kb = next(int(l.split()[1]) for l in open("/proc/meminfo") if l.startswith("MemTotal"))
print(json.dumps({"engine": sys.argv[1], "version": sys.argv[2], "cpu_model": model,
                  "cpu_cores": os.cpu_count(), "memory_gb": round(mem_kb / 1024 / 1024, 1)}))
PY
curl -s localhost:9200
echo "ready $ENGINE $VERSION"
