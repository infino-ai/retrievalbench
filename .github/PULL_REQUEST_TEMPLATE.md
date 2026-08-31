## What this changes

<!-- One or two sentences. If it changes a published number, say so here. -->

## Checklist

- [ ] `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` pass.
- [ ] No `path =` dependency on the engine. `Cargo.toml` still pins `infino` and `infino-bench-utils` by `rev`.

### If this changes published results

- [ ] Results were regenerated with `scripts/publish_results.sh` on a clean tree — no hand-edited numbers, no bypassed dirty-tree check.
- [ ] Every row in a given table comes from **one run on one host**.
- [ ] The run was made with `RUST_LOG=infino=warn` and produced no fallback or drain-decline warnings.
- [ ] Recall is reported alongside latency for anything approximate.
- [ ] The engine commit, host, and command are recorded in the run's `run.json`.

### If this touches a peer engine's adapter

- [ ] The engine is reached through its own public API, as its documentation describes.
- [ ] Its documented build settings are used (for FAISS: built via `scripts/build_faiss.sh`, so FastScan is not silently scalar).
- [ ] Its version is pinned by SHA or release, and recorded.

### If this touches `.github/`

- [ ] No workflow gained a `pull_request` or `pull_request_target` trigger. Those workflows hold cloud OIDC credentials; the absence of that trigger is what keeps them out of reach of fork PRs — see [SECURITY.md](../SECURITY.md).
- [ ] No job's access to secrets was widened, and no cloud project, bucket, or service account was hardcoded.
- [ ] Dispatch workflows still serialize and still refuse to cancel mid-flight (a cancelled run leaks a billing VM).

### Before merging

- [ ] This repository is public. Nothing in the diff — code, comments, or commit message — names a non-public repository or uses internal shorthand. Describe things on their own terms.
- [ ] No AI-attribution trailer on the commits, no generated-by footer in this description.
