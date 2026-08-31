## What this changes

<!-- One or two sentences. If it changes a published number, say so. -->

## Checklist

- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` pass.
- [ ] `Cargo.toml` still pins `infino` and `infino-bench-utils` by `rev` — no `path =` dependency.

**If this changes published results:**

- [ ] Regenerated with `scripts/publish_results.sh` on a clean tree — no hand-edited numbers, no bypassed dirty-tree check.
- [ ] Every row in a table comes from one run on one host.
- [ ] Run made with `RUST_LOG=infino=warn`, with no fallback or drain-decline warnings.

**If this touches a peer engine's adapter:**

- [ ] Reached through the engine's own public API, with its documented build settings (FAISS built via `scripts/build_faiss.sh`, so FastScan isn't silently scalar).
- [ ] Version pinned by SHA or release, and recorded.

**If this touches `.github/`:**

- [ ] No workflow gained a `pull_request` or `pull_request_target` trigger, and no job's access to secrets was widened. See [SECURITY.md](../SECURITY.md).
- [ ] Dispatch workflows still serialize and still refuse to cancel mid-flight.

**Before merging:**

- [ ] This repo is public. Nothing in the diff — code, comments, or commit message — names a non-public repository or uses internal shorthand.
