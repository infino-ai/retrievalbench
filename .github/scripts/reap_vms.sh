#!/usr/bin/env bash
# Safety net for leaked bench VMs (partial provision, runner death, failed
# teardown) that the in-job teardown can't catch. For the selected $CLOUD and
# each allowed prefix, deletes:
#   1. Instances whose `delete-after` is in the past — a healthy run (job
#      timeout <= the tag's horizon) tears itself down first, so a still-present
#      expired instance is leaked. Untagged matching instances count as leaked.
#   2. Orphaned dependents (Azure nic/disk/pip/nsg/vnet, AWS sg/key, GCP
#      firewall) whose base instance is gone.
# Scoped STRICTLY to $PREFIXES (an explicit allowlist). Anything not listed is
# never touched — in particular bench-vm-* (infino's own Benchmark run).
set -euo pipefail
CLOUD="${CLOUD:-azure}"
PREFIXES="${PREFIXES:-vdbbench- clickbench-}"
RG="${RG:-rg-infino-bench}"          # Azure resource group
AWS_REGION_R="${AWS_REGION_R:-us-east-1}"
now="$(date -u +%s)"

# True if $1 is an unexpired epoch (in the future); false if past/empty/None.
epoch_in_future() {
  case "$1" in ''|None) return 1 ;; esac
  [[ "$1" =~ ^[0-9]+$ ]] && [ "$now" -lt "$1" ]
}

# --- Azure ------------------------------------------------------------------
az_del_vm_and_deps() {
  local n="$1"
  echo "reap VM: $n"
  az vm delete -g "$RG" -n "$n" --yes 2>/dev/null || true
  az network nic delete -g "$RG" -n "${n}VMNic" 2>/dev/null || true
  az disk delete -g "$RG" -n "${n}-osdisk" --yes 2>/dev/null || true
  az network public-ip delete -g "$RG" -n "${n}-pip" 2>/dev/null || true
  az network nsg delete -g "$RG" -n "${n}-nsg" 2>/dev/null || true
  az network vnet delete -g "$RG" -n "${n}-vnet" 2>/dev/null || true
}

reap_azure() {
  local pfx="$1"
  echo "== expired ${pfx}* VMs in $RG =="
  az vm list -g "$RG" \
    --query "[?starts_with(name,'${pfx}')].[name, tags.\"delete-after\"]" -o tsv 2>/dev/null |
    while IFS=$'\t' read -r name del; do
      [ -n "$name" ] || continue
      exp="$(date -u -d "$del" +%s 2>/dev/null || echo 0)"   # Azure tag is ISO-8601
      if epoch_in_future "$exp"; then
        echo "keep $name (expires $del)"; continue
      fi
      az_del_vm_and_deps "$name"
    done

  echo "== orphaned ${pfx}* dependents (no live VM) =="
  local live
  live="$(az vm list -g "$RG" --query "[?starts_with(name,'${pfx}')].name" -o tsv 2>/dev/null)"
  az resource list -g "$RG" \
    --query "[?starts_with(name,'${pfx}')].[name, id]" -o tsv 2>/dev/null |
    while IFS=$'\t' read -r name id; do
      [ -n "$id" ] || continue
      base="$name"
      for suf in VMNic -osdisk -pip -nsg -vnet -subnet; do base="${base%"$suf"}"; done
      printf '%s\n' "$live" | grep -qxF "$base" && continue
      echo "reap orphan: $name"
      az resource delete --ids "$id" 2>/dev/null || true   # stragglers clear next run
    done
}

# --- AWS --------------------------------------------------------------------
# Instance Name == SG name == key-pair name == "${prefix}${run_id}". SGs and
# key-pairs don't bill, but delete them for hygiene once no instance uses them.
reap_aws() {
  local pfx="$1"
  echo "== expired ${pfx}* instances in $AWS_REGION_R =="
  aws ec2 describe-instances --region "$AWS_REGION_R" \
    --filters "Name=tag:Name,Values=${pfx}*" \
              "Name=instance-state-name,Values=pending,running,stopping,stopped" \
    --query "Reservations[].Instances[].[InstanceId, (Tags[?Key=='Name'].Value)[0], (Tags[?Key=='delete-after'].Value)[0]]" \
    --output text 2>/dev/null |
    while IFS=$'\t' read -r iid name del; do
      [ -n "$iid" ] || continue
      if epoch_in_future "$del"; then
        echo "keep $name (expires epoch $del)"; continue
      fi
      echo "reap instance: $name ($iid)"
      aws ec2 terminate-instances --region "$AWS_REGION_R" --instance-ids "$iid" >/dev/null 2>&1 || true
      aws ec2 wait instance-terminated --region "$AWS_REGION_R" --instance-ids "$iid" 2>/dev/null || true
      aws ec2 delete-security-group --region "$AWS_REGION_R" --group-name "$name" 2>/dev/null || true
      aws ec2 delete-key-pair --region "$AWS_REGION_R" --key-name "$name" 2>/dev/null || true
    done

  echo "== orphaned ${pfx}* sg/key (no live instance) =="
  local live
  live="$(aws ec2 describe-instances --region "$AWS_REGION_R" \
    --filters "Name=tag:Name,Values=${pfx}*" \
              "Name=instance-state-name,Values=pending,running,stopping,stopped" \
    --query "Reservations[].Instances[].(Tags[?Key=='Name'].Value)[0]" --output text 2>/dev/null | tr '\t' '\n')"
  aws ec2 describe-security-groups --region "$AWS_REGION_R" \
    --filters "Name=group-name,Values=${pfx}*" --query "SecurityGroups[].GroupName" --output text 2>/dev/null |
    tr '\t' '\n' | while read -r sg; do
      [ -n "$sg" ] || continue
      printf '%s\n' "$live" | grep -qxF "$sg" && continue
      echo "reap orphan sg/key: $sg"
      aws ec2 delete-security-group --region "$AWS_REGION_R" --group-name "$sg" 2>/dev/null || true
      aws ec2 delete-key-pair --region "$AWS_REGION_R" --key-name "$sg" 2>/dev/null || true
    done
}

# --- GCP --------------------------------------------------------------------
# Instance name == firewall-rule name == "${prefix}${run_id}".
reap_gcp() {
  local pfx="$1"
  echo "== expired ${pfx}* instances =="
  gcloud compute instances list --filter="name~^${pfx}" \
    --format="value(name, labels.delete-after, zone)" 2>/dev/null |
    while IFS=$'\t' read -r name del zone; do
      [ -n "$name" ] || continue
      if epoch_in_future "$del"; then
        echo "keep $name (expires epoch $del)"; continue
      fi
      echo "reap instance: $name ($zone)"
      gcloud compute instances delete "$name" --zone "$zone" --quiet 2>/dev/null || true
      gcloud compute firewall-rules delete "$name" --quiet 2>/dev/null || true
    done

  echo "== orphaned ${pfx}* firewall rules (no live instance) =="
  local live
  live="$(gcloud compute instances list --filter="name~^${pfx}" --format="value(name)" 2>/dev/null)"
  gcloud compute firewall-rules list --filter="name~^${pfx}" --format="value(name)" 2>/dev/null |
    while read -r fw; do
      [ -n "$fw" ] || continue
      printf '%s\n' "$live" | grep -qxF "$fw" && continue
      echo "reap orphan firewall: $fw"
      gcloud compute firewall-rules delete "$fw" --quiet 2>/dev/null || true
    done
}

case "$CLOUD" in
  azure) reap=reap_azure ;;
  aws)   reap=reap_aws ;;
  gcp)   reap=reap_gcp ;;
  *) echo "unknown CLOUD=$CLOUD" >&2; exit 1 ;;
esac

for pfx in $PREFIXES; do
  "$reap" "$pfx"
done
echo "== done: $CLOUD swept for $PREFIXES =="
