# Security Policy

## Reporting a vulnerability

**Please do not open a public issue for security vulnerabilities.**

Report security issues privately through GitHub's private vulnerability
reporting: open the **Security** tab on
<https://github.com/infino-ai/retrievalbench> and choose **Report a
vulnerability**.

Please include enough detail to reproduce the issue (affected commit, steps,
and impact). We aim to acknowledge reports within a few business days and will
keep you updated on remediation. Coordinated disclosure is appreciated — we
will credit reporters who wish to be named.

If the issue is in the retrieval engine itself rather than in this harness,
report it on [`infino`](https://github.com/infino-ai/infino/security) instead —
that is where a fix would ship.

## Scope

This repository is a benchmark harness. It publishes no package and serves no
traffic, so its security surface is its automation rather than its code:

- **The workflows and composite actions under `.github/`.** They federate into
  AWS, Azure, and GCP over OIDC and create and destroy virtual machines.
- **`scripts/`**, which those workflows execute.
- **The committed results under `results/`** and the run metadata in each
  `run.json`, if any of it ever discloses more than it should.

The benchmark code under `benches/` measures third-party engines in-process.
A crash, hang, or wrong number there is a correctness bug — please open an
ordinary issue for it.

## A note on the cloud credentials

Every workflow in this repository is triggered by `workflow_dispatch`,
`workflow_call`, or `schedule` — never by `pull_request`. That is deliberate
and load-bearing: it is what keeps the cloud identities out of reach of pull
requests from forks. A change that adds a `pull_request` or
`pull_request_target` trigger to a workflow with `id-token: write`, or that
grants an existing job access to a secret it did not previously hold, is a
security change and should be reviewed as one.

If you believe a workflow can be made to run attacker-controlled code with
those credentials, that is a vulnerability — report it privately as above.

## Supported versions

This harness is not released or versioned. Fixes land on `main`; there are no
maintenance branches.
