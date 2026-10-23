#!/usr/bin/env bash
# Run idbench and write its tables to results/<date>-<machine>.md, stamped
# with what produced them: the machine, the toolchain, the exact Infino
# commit, this repository's commit, and the machine's load average when the
# run started.
#
#   ./run.sh <machine>        e.g. ./run.sh gcp-c3-highcpu-8
#
# Refuses to run with uncommitted changes under idbench/, so every result
# file names a commit that reproduces it.
set -euo pipefail
cd "$(dirname "$0")"

machine=${1:?usage: ./run.sh <machine>, e.g. gcp-c3-highcpu-8}
if [ -n "$(git status --porcelain -- .)" ]; then
    echo "run.sh: idbench/ has uncommitted changes; commit them first" >&2
    exit 1
fi

cargo build --release --locked

if [ "$(uname -s)" = Linux ]; then
    cpu=$(lscpu | sed -n 's/^Model name: *//p')
    mem_gib=$(awk '/MemTotal/ { printf "%.0f", $2 / 1048576 }' /proc/meminfo)
    os="$(. /etc/os-release && echo "$PRETTY_NAME"), kernel $(uname -r)"
else
    cpu=$(sysctl -n machdep.cpu.brand_string)
    mem_gib=$(( $(sysctl -n hw.memsize) / 1073741824 ))
    os="$(sw_vers -productName) $(sw_vers -productVersion)"
fi
load() { uptime | sed 's/.*load average[s]*: //'; }
infino=$(grep -A2 '^name = "infino"$' Cargo.lock | sed -n 's/^source = "\(.*\)"$/\1/p')

mkdir -p results
out=results/$(date -u +%Y-%m-%d)-$machine.md
load_before=$(load)
started=$(date -u '+%Y-%m-%d %H:%M UTC')
tables=$(./target/release/idbench | tail -n +2)

{
    echo "# idbench results, $machine"
    echo
    echo "| | |"
    echo "|---|---|"
    echo "| started | $started |"
    echo "| machine | $machine |"
    echo "| cpu | $cpu, $(getconf _NPROCESSORS_ONLN) logical cpus |"
    echo "| memory | $mem_gib GiB |"
    echo "| os | $os |"
    echo "| rustc | $(rustc --version) |"
    echo "| infino | \`$infino\` |"
    echo "| retrievalbench | \`$(git rev-parse HEAD)\` |"
    echo "| command | \`./run.sh $machine\` |"
    echo "| load average at start (1, 5, 15 min) | $load_before |"
    echo "$tables"
} > "$out"
echo "wrote $out"
