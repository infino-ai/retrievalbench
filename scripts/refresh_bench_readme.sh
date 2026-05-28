#!/usr/bin/env bash
#
# refresh_bench_readme.sh
#
# Nightly bench job. Two phases, run from the workspace umbrella dir
# (so both sibling repos are visible):
#
#   Phase A (infino):
#     Stash any WIP, fetch origin/main, detach HEAD at origin/main,
#     run `cargo bench`. No commit, no push. Local `main` branch is
#     never touched (detached HEAD).
#
#   Phase B (retrievalbench), only if Phase A succeeds:
#     Stash any WIP, fetch origin/main, create/reset
#     `bench/auto-refresh` from origin/main, run `cargo bench --bench
#     fts` and `--bench vector` with INFINO_BENCH_UPDATE_README=1,
#     commit the resulting `benches/README.md` diff, force-push, and
#     either open a PR or rely on the force-push to update the
#     existing rolling PR.
#
# Each phase's original branch and stash are restored on every exit
# path via a single LIFO trap. On non-zero exit, a GitHub issue is
# opened with the tail of the log.
#
# Safe under cron: minimal-PATH-tolerant, single-instance via flock,
# all output tee'd to a dated log file.

set -euo pipefail

# Cron's PATH is minimal; make cargo and gh resolvable.
export PATH="$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"

# Benches mmap many files in parallel and exhaust the default 1024
# soft open-files limit (EMFILE / "Too many open files" panic in the
# vector bench builder). Raise to 65536, falling back to the hard
# limit if it's lower than that.
ulimit -n 65536 2>/dev/null || ulimit -n "$(ulimit -Hn)" 2>/dev/null || true

RB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_DIR="$(cd "$RB_DIR/.." && pwd)"
INFINO_DIR="$WORKSPACE_DIR/infino"

LOG_DIR="$HOME/.cache/retrievalbench-bench"
mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/$(date -u +%Y%m%dT%H%M%SZ).log"

exec > >(tee -a "$LOG_FILE") 2>&1

echo "[$(date -Is)] starting nightly bench"
echo "  workspace: $WORKSPACE_DIR"
echo "  infino:    $INFINO_DIR"
echo "  rb:        $RB_DIR"

cd "$WORKSPACE_DIR"

LOCK_FILE="/tmp/retrievalbench-bench.lock"
exec 9>"$LOCK_FILE"
if ! flock -n 9; then
  echo "another run is in progress (lock held on $LOCK_FILE); exiting"
  exit 0
fi

# Phase state. All vars initialized before the trap so the trap can
# reference them safely under `set -u`.
PHASE="init"
INFINO_ORIG_BRANCH=""
INFINO_STASHED=0
RB_ORIG_BRANCH=""
RB_STASHED=0

report_failure() {
  local rc="$1"
  local title
  title="bench: nightly run failed in phase '$PHASE' $(date -u +%Y-%m-%d)"
  local body
  body=$(
    printf '%s\n\n%s\n\n%s\n\n```\n%s\n```\n' \
      "Nightly bench on $(hostname) exited with code $rc during phase '$PHASE'." \
      "Log on $(hostname): \`$LOG_FILE\`" \
      "Last 80 lines:" \
      "$(tail -n 80 "$LOG_FILE")"
  )
  gh issue create \
    --repo infino-ai/retrievalbench \
    --title "$title" \
    --body "$body" \
    || echo "WARNING: failed to open failure issue via gh"
}

restore_repo() {
  local dir="$1"
  local orig_branch="$2"
  local stashed="$3"
  if [ -z "$orig_branch" ]; then
    return 0
  fi
  echo "  restoring $dir -> $orig_branch (stashed=$stashed)"
  git -C "$dir" checkout "$orig_branch" \
    || echo "WARNING: could not checkout $orig_branch in $dir"
  if [ "$stashed" = 1 ]; then
    git -C "$dir" stash pop \
      || echo "WARNING: git stash pop failed in $dir; check 'git -C $dir stash list'"
  fi
}

restore_state() {
  local rc=$?
  echo "[$(date -Is)] restoring state (exit=$rc, phase=$PHASE)"
  # LIFO: restore retrievalbench (touched second) before infino.
  restore_repo "$RB_DIR" "$RB_ORIG_BRANCH" "$RB_STASHED"
  restore_repo "$INFINO_DIR" "$INFINO_ORIG_BRANCH" "$INFINO_STASHED"
  if [ "$rc" -ne 0 ]; then
    report_failure "$rc"
  fi
}

trap restore_state EXIT

# Returns 0 if the repo has any uncommitted state (staged, unstaged, or
# untracked-but-not-ignored), 1 otherwise.
repo_dirty() {
  local dir="$1"
  if ! git -C "$dir" diff --quiet \
    || ! git -C "$dir" diff --cached --quiet \
    || [ -n "$(git -C "$dir" ls-files --others --exclude-standard)" ]; then
    return 0
  fi
  return 1
}

# ---------------------------------------------------------------------
# Phase A: infino — bench against latest origin/main, no commit, no PR.
# ---------------------------------------------------------------------
PHASE="infino"
echo "[$(date -Is)] phase: $PHASE"

INFINO_ORIG_BRANCH="$(git -C "$INFINO_DIR" symbolic-ref --short HEAD)"

if repo_dirty "$INFINO_DIR"; then
  echo "stashing infino WIP"
  git -C "$INFINO_DIR" stash push -u -m "auto-bench-refresh $(date -Is)"
  INFINO_STASHED=1
fi

git -C "$INFINO_DIR" fetch origin main
# Detached HEAD so we never overwrite local main's history.
git -C "$INFINO_DIR" checkout --detach origin/main

( cd "$INFINO_DIR" && cargo bench )

# ---------------------------------------------------------------------
# Phase B: retrievalbench — bench against latest origin/main, update
# README, force-push to bench/auto-refresh, open or update rolling PR.
# ---------------------------------------------------------------------
PHASE="retrievalbench"
echo "[$(date -Is)] phase: $PHASE"

RB_ORIG_BRANCH="$(git -C "$RB_DIR" symbolic-ref --short HEAD)"

if repo_dirty "$RB_DIR"; then
  echo "stashing retrievalbench WIP"
  git -C "$RB_DIR" stash push -u -m "auto-bench-refresh $(date -Is)"
  RB_STASHED=1
fi

git -C "$RB_DIR" fetch origin main
git -C "$RB_DIR" checkout -B bench/auto-refresh origin/main

(
  cd "$RB_DIR"
  INFINO_BENCH_UPDATE_README=1 cargo bench --bench fts
  INFINO_BENCH_UPDATE_README=1 cargo bench --bench vector
)

if git -C "$RB_DIR" diff --quiet -- benches/README.md; then
  echo "no README changes after bench run; nothing to PR"
  exit 0
fi

git -C "$RB_DIR" add benches/README.md
git -C "$RB_DIR" commit -m "bench: refresh README results ($(date -u +%Y-%m-%d))"
git -C "$RB_DIR" push --force-with-lease origin bench/auto-refresh

existing_pr="$(
  gh pr list \
    --repo infino-ai/retrievalbench \
    --head bench/auto-refresh \
    --state open \
    --json number \
    --jq '.[0].number' \
  || true
)"

if [ -z "${existing_pr:-}" ]; then
  gh pr create \
    --repo infino-ai/retrievalbench \
    --base main \
    --head bench/auto-refresh \
    --title "bench: nightly README refresh $(date -u +%Y-%m-%d)" \
    --body "Automated nightly run on $(hostname). Force-pushed to \`bench/auto-refresh\`; this PR rolls forward each night."
else
  echo "existing PR #$existing_pr updated via force-push"
fi
