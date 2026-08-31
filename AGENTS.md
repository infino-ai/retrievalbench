# RetrievalBench — notes for AI agents

Read [`CONTRIBUTING.md`](CONTRIBUTING.md) first. It covers prerequisites, the
build, how to run a benchmark, how results are published, and the pull-request
workflow. This file covers only what isn't in there: the traps that are not
obvious from the code, and the boundaries that matter here.

## What this repository is

A benchmark harness that measures [`infino`](https://github.com/infino-ai/infino)
head-to-head against other retrieval engines, and publishes the numbers. It
ships no package and serves no traffic.

Its product is **trust in a number**. That reframes what "correct" means: code
that compiles, runs, and produces a plausible figure can still be entirely
wrong, and nothing in the build will tell you. Optimize for a reader being able
to reproduce and believe the result, not for the harness being elegant.

## The golden rule

**A number you cannot reproduce is worse than no number.** If you cannot say
which engine commit, which host, which corpus, and which command produced a
figure, it does not get published. When in doubt, re-run rather than reason
about what the number probably would have been.

Never invent, extrapolate, interpolate, or "correct" a measurement. If a run
is missing, the honest output is that it is missing.

## Traps

**The engine is pinned by `rev`, not by path or branch.** Both `infino` and
`infino-bench-utils` in `Cargo.toml` point at an explicit SHA, and each
committed `run.json` records the SHA it measured. Swapping in a `path =`
dependency while developing is fine locally; committing one silently detaches
every published row from an identifiable engine. Never commit that swap.

**Results are generated, never written.** `results/README.md` comes out of
`scripts/publish_inprocess.py`, reading the committed JSON. Editing a table by
hand produces markdown that disagrees with the data beside it, and nothing
checks the two against each other. Refresh means re-run, then regenerate.

**`publish_results.sh` refuses a dirty tree on purpose.** That refusal is the
provenance guarantee, not a nuisance. Do not bypass it, and do not add a flag
that would let someone else bypass it.

**A silent fallback looks exactly like a good run.** Engine diagnostics — drain
declines, serving-mode fallbacks — are `tracing` events, and with no subscriber
installed they vanish. Run with `RUST_LOG=infino=warn` and read the output. A
run that quietly served from a different mode than the one the row claims is
the single worst artifact this repository can produce.

**FAISS without `-march=native` is a wrong number, not a slow one.** FastScan
falls back to scalar code silently. `scripts/build_faiss.sh` passes the flag;
use it rather than a system FAISS.

**Engine behaviour comes from `infino.yaml` only.** The `INFINO_BENCH_*`
environment variables size a run — document count, store, cost assumptions.
They do not change what the engine does. If you believe you have found one
that does, that is a bug to report, not a knob to use.

**Rows in one table must come from one run on one host.** Assembling a
comparison from separate runs, or from different machines, produces a table
that is wrong in a way no reviewer can see. `run.json` records the host
precisely so this stays checkable.

## CI and cloud credentials

Every workflow triggers on `workflow_dispatch`, `workflow_call`, or `schedule`.
None triggers on `pull_request`, and that is load-bearing: these workflows
federate into AWS, Azure, and GCP over OIDC and create real virtual machines,
and the missing pull-request trigger is what keeps those identities unreachable
from a fork's pull request.

Do not add a `pull_request` or `pull_request_target` trigger to any workflow
holding `id-token: write`, and do not widen a job's access to secrets, as an
incidental part of some other change. That is a security change and needs to be
reviewed as one — see [`SECURITY.md`](SECURITY.md).

The dispatch workflows serialize and refuse to cancel mid-flight because a
cancelled run leaks a virtual machine that keeps billing. `vm-reaper.yml` is the
backstop for what still escapes. Preserve both behaviours; if you touch
concurrency configuration, say why in the pull request.

Cloud project identifiers, buckets, and service accounts come from repository
variables and secrets. Never hardcode one, and never echo a secret into a log.

## Writing for a public repository

This repository is public. Everything in it — code, comments, commit messages,
pull-request text — is read by people outside Infino.

- Reference only public repositories and published artifacts. The engine,
  the ClickBench and Search Benchmark ports, and the VectorDBBench client are
  all public and may be linked freely.
- Name things on their own terms. Internal shorthand — abbreviations, numbered
  labels, ticket-style tags — is meaningless to a reader here. If a concept
  needs a name, give it a descriptive one.
- Do not add AI-attribution trailers to commits, and do not add generated-by
  footers to pull-request or issue descriptions. Commit metadata and pull
  requests read as human-authored.

## Measuring other people's engines

The comparators are real projects, much of it volunteer-maintained, and we
publish numbers about them. Reach each engine through its own public API as its
documentation describes; give it its documented build settings and a fair
tuning pass; pin its version; and report recall next to latency for anything
approximate.

When Infino loses a row, publish the row. Do not tune only Infino's side, do not
quietly drop an unfavourable comparator, and do not reword a caption to blur a
result. If you notice a configuration that looks unfair to a peer, fix it or
raise it — that is a correctness bug here, not a competitive question.

## Sources of truth

Where this file overlaps with configuration, the configuration wins:
`Cargo.toml` for dependency pins and features, `rust-toolchain.toml` for the
toolchain, `.github/workflows/` for what CI actually does, and each run's
`run.json` for what a published number actually measured.
