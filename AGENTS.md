# RetrievalBench — notes for AI agents

Read [`CONTRIBUTING.md`](CONTRIBUTING.md) first: prerequisites, build, how to
run a benchmark, how results get published, and the pull-request workflow.
This file covers the traps that aren't obvious from the code.

This repository measures [`infino`](https://github.com/infino-ai/infino)
against other retrieval engines and publishes the numbers. Code that compiles
and produces a plausible figure can still be completely wrong, and nothing in
the build will tell you. Optimize for reproducibility.

Never invent or extrapolate a measurement. If a run is missing, say so.

## Traps

**The engine is pinned by `rev`, not by path or branch.** Both `infino` and
`infino-bench-utils` in `Cargo.toml` point at an explicit SHA, and each
committed `run.json` records the SHA it measured. Swapping in a `path =`
dependency is fine locally, but committing one detaches every published row
from an identifiable engine.

**Results are generated, never written.** `results/README.md` comes out of
`scripts/publish_inprocess.py`, reading the committed JSON. Editing a table by
hand produces markdown that disagrees with the data beside it, and nothing
checks the two against each other. Refresh means re-run, then regenerate.

**`publish_results.sh` refuses a dirty tree.** That's the provenance
guarantee. Don't bypass it, and don't add a flag that lets someone else bypass
it.

**A silent fallback looks exactly like a good run.** Engine diagnostics — drain
declines, serving-mode fallbacks — are `tracing` events, and with no subscriber
installed they vanish. Run with `RUST_LOG=infino=warn` and read the output.

**Build FAISS with `scripts/build_faiss.sh`, not a system FAISS.** It passes
`-march=native`; without that, FastScan silently runs scalar and the row is
wrong.

**Engine behavior comes from `infino.yaml` only.** The `INFINO_BENCH_*`
variables size a run — document count, store, cost assumptions. They don't
change what the engine does. If you find one that does, that's a bug; report
it.

**Rows in one table must come from one run on one host.** Assembling a
comparison from separate runs, or different machines, produces a table nobody
can see is wrong. `run.json` records the host so it stays checkable.

## CI and cloud credentials

Every workflow triggers on `workflow_dispatch`, `workflow_call`, or
`schedule`. None triggers on `pull_request`, which keeps the AWS/Azure/GCP
OIDC identities unreachable from a fork's pull request. Don't add such a
trigger to a workflow holding `id-token: write`, and don't widen a job's
access to secrets, as an incidental part of some other change. See
[`SECURITY.md`](SECURITY.md).

Dispatch workflows serialize and are never cancelled mid-flight: a cancelled
run orphans a VM that keeps billing. `vm-reaper.yml` cleans up what still
slips through. Keep both. If you touch concurrency config, say why in the pull
request.

Cloud projects, buckets, and service accounts come from repository variables
and secrets. Don't hardcode one, and don't echo a secret into a log.

## Writing for a public repository

Everything here — code, comments, commit messages, pull-request text — is read
by people outside Infino.

- Reference only public repositories and published artifacts. The engine, the
  ClickBench and Search Benchmark ports, and the VectorDBBench client are all
  public and can be linked freely.
- Name things on their own terms. Internal shorthand — abbreviations, numbered
  labels, ticket-style tags — means nothing to a reader here.
- No AI-attribution trailers on commits, no generated-by footers on pull
  requests or issues.

## Measuring other people's engines

The operational rules are in
[`CONTRIBUTING.md`](CONTRIBUTING.md#measuring-a-peer-engine-fairly): own public
API, documented build settings, pinned versions, recall reported next to
latency. When Infino loses a row, publish the row. If a peer's configuration
looks unfair, fix it or open an issue. It's a correctness bug.

## Sources of truth

Where this file overlaps with configuration, the configuration wins:
`Cargo.toml` for dependency pins and features, `rust-toolchain.toml` for the
toolchain, `.github/workflows/` for what CI actually does, and each run's
`run.json` for what a published number measured.
