---
name: Bug report
about: Something in the harness is wrong, crashes, or produces a number you cannot reproduce
labels: bug
---

**What happened**

<!-- Include the exact command. -->

**What you expected**

**Environment**

- OS / CPU:
- `rustc --version`:
- Commit of this repository:
- Built with `--features faiss`? If so, was FAISS built via `scripts/build_faiss.sh`?

**Output**

<!--
Please re-run with RUST_LOG=infino=warn and include the output. Engine
diagnostics are tracing events; without a subscriber they are dropped silently,
and a run that quietly fell back to a different serving mode looks exactly like
a good one.
-->

```
```
