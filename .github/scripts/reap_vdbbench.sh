#!/usr/bin/env bash
# Safety net for leaked VectorDBBench VMs (partial provision, runner death,
# failed teardown) that the in-job teardown can't catch. Deletes, in
# rg-infino-bench and scoped strictly to `vdbbench-*`:
#   1. VMs whose `delete-after` tag is in the past — a healthy run (job
#      timeout <= 3h == the tag's horizon) tears itself down first, so a
#      still-present expired VM is leaked. Untagged vdbbench VMs count as leaked.
#   2. Orphaned dependents (nic/disk/pip/nsg/vnet) whose base VM is gone.
# Never touches other prefixes (e.g. bench-vm-*, clickbench-*).
set -euo pipefail
RG="${RG:-rg-infino-bench}"
now="$(date -u +%s)"

live_after_reap() {
  az vm list -g "$RG" --query "[?starts_with(name,'vdbbench-')].name" -o tsv 2>/dev/null
}

del_vm_and_deps() {
  local n="$1"
  echo "reap VM: $n"
  az vm delete -g "$RG" -n "$n" --yes 2>/dev/null || true
  az network nic delete -g "$RG" -n "${n}VMNic" 2>/dev/null || true
  az disk delete -g "$RG" -n "${n}-osdisk" --yes 2>/dev/null || true
  az network public-ip delete -g "$RG" -n "${n}-pip" 2>/dev/null || true
  az network nsg delete -g "$RG" -n "${n}-nsg" 2>/dev/null || true
  az network vnet delete -g "$RG" -n "${n}-vnet" 2>/dev/null || true
}

echo "== expired vdbbench VMs in $RG =="
az vm list -g "$RG" \
  --query "[?starts_with(name,'vdbbench-')].[name, tags.\"delete-after\"]" -o tsv 2>/dev/null |
  while IFS=$'\t' read -r name del; do
    [ -n "$name" ] || continue
    if [ -n "$del" ] && [ "$del" != "None" ]; then
      exp="$(date -u -d "$del" +%s 2>/dev/null || echo 0)"
      if [ "$now" -lt "$exp" ]; then
        echo "keep $name (expires $del)"
        continue
      fi
    fi
    del_vm_and_deps "$name"
  done

echo "== orphaned vdbbench dependents (no live VM) =="
live="$(live_after_reap)"
az resource list -g "$RG" \
  --query "[?starts_with(name,'vdbbench-')].[name, id]" -o tsv 2>/dev/null |
  while IFS=$'\t' read -r name id; do
    [ -n "$id" ] || continue
    base="$name"
    for suf in VMNic -osdisk -pip -nsg -vnet -subnet; do base="${base%"$suf"}"; done
    if printf '%s\n' "$live" | grep -qxF "$base"; then
      continue  # base VM still alive → not an orphan
    fi
    echo "reap orphan: $name"
    az resource delete --ids "$id" 2>/dev/null || true  # stragglers clear next run
  done

echo "== done; remaining vdbbench resources: $(az resource list -g "$RG" --query "length([?starts_with(name,'vdbbench-')])" -o tsv 2>/dev/null) =="
