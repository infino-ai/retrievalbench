# Security Policy

## Reporting a vulnerability

**Please don't open a public issue for security vulnerabilities.**

Report privately through GitHub's private vulnerability reporting: open the
**Security** tab on <https://github.com/infino-ai/retrievalbench> and choose
**Report a vulnerability**.

Include enough detail to reproduce (affected commit, steps, impact). We aim to
acknowledge within a few business days and will keep you updated. Coordinated
disclosure is appreciated, and we'll credit reporters who want to be named.

If the issue is in the retrieval engine rather than this harness, report it on
[`infino`](https://github.com/infino-ai/infino/security) instead — that's
where a fix would ship.

## Scope

The security surface here is the automation, not the benchmark code:

- **The workflows and composite actions under `.github/`.** They federate into
  AWS, Azure, and GCP over OIDC and create and destroy virtual machines.
- **`scripts/`**, which those workflows execute.
- **The committed results under `results/`** and the run metadata in each
  `run.json`, if any of it discloses more than it should.

The code under `benches/` measures third-party engines in-process. A crash,
hang, or wrong number there is a correctness bug; please open an ordinary
issue for it.

## Cloud credentials

Every workflow here is triggered by `workflow_dispatch`, `workflow_call`, or
`schedule`, never by `pull_request`. That keeps the cloud identities out of
reach of pull requests from forks. A change that adds a `pull_request` or
`pull_request_target` trigger to a workflow with `id-token: write`, or that
grants an existing job a secret it didn't hold before, is a security change
and should be reviewed as one.

If you think a workflow can be made to run attacker-controlled code with those
credentials, report it privately as above.

## Supported versions

This harness isn't released or versioned. Fixes land on `main`; there are no
maintenance branches.
