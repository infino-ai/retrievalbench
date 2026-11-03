#!/usr/bin/env bash
# Runs on the Rally runner VM (as the admin user, with sudo): puts Rally's track data on the
# local SSD and installs Infino's Rally fork (infinohq/rally, branch `fork`), which adds the
# `database_type` client option for OpenSearch and Elasticsearch targets. The config is the
# one the AWS harness wrote.
# Usage: runner-setup.sh
set -euo pipefail

RALLY=/mnt/rally
LOCAL_SSD=/dev/disk/by-id/google-local-nvme-ssd-0
RALLY_REPO=${RALLY_REPO:-https://github.com/infinohq/rally.git}
RALLY_BRANCH=${RALLY_BRANCH:-fork}

sudo DEBIAN_FRONTEND=noninteractive apt-get update -y >/dev/null
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y git jq pbzip2 python3-venv python3-dev build-essential >/dev/null

if [ -e "$LOCAL_SSD" ]; then
  sudo mkfs.ext4 -F -q "$LOCAL_SSD"
  sudo mkdir -p "$RALLY"
  sudo mount "$LOCAL_SSD" "$RALLY"
else
  echo "warning: no local SSD at $LOCAL_SSD; using the boot disk" >&2
  sudo mkdir -p "$RALLY"
fi
sudo chown "$(id -u):$(id -g)" "$RALLY"
mkdir -p "$RALLY/results" "$RALLY/.rally/benchmarks/data"

git clone --quiet --branch "$RALLY_BRANCH" "$RALLY_REPO" "$RALLY/rally-fork"
python3 -m venv "$RALLY/venv"
"$RALLY/venv/bin/pip" install --quiet --upgrade pip
"$RALLY/venv/bin/pip" install --quiet -e "$RALLY/rally-fork"

mkdir -p "$HOME/.rally"
cat > "$HOME/.rally/rally.ini" <<EOF
[meta]
config.version = 17

[system]
env.name = rally-benchmark

[node]
root.dir = $RALLY/.rally/benchmarks
src.root.dir = $RALLY/.rally/benchmarks/src

[source]
remote.repo.url = https://github.com/elastic/elasticsearch.git
elasticsearch.src.subdir = elasticsearch

[benchmarks]
local.dataset.cache = $RALLY/.rally/benchmarks/data

[reporting]
datastore.type = in-memory

[tracks]
default.url = https://github.com/elastic/rally-tracks
eventdata.url = https://github.com/elastic/rally-eventdata-track

[teams]
default.url = https://github.com/elastic/rally-teams

[defaults]
preserve_benchmark_candidate = False

[distributions]
release.cache = true

[telemetry]
devices =
EOF
"$RALLY/venv/bin/esrally" --version
echo "ready runner"
