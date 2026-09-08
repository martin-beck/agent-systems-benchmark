# Reproducibility guide

A reproducible ASB comparison begins with immutable inputs, not just the same
command line. Preserve:

- ASB source commit and clean tree, pinned Rust toolchain, Cargo lockfiles, and
  verified agent executable digest;
- experiment content address, model/provider settings, tool policy, workload and
  scorer revisions, image/dependency digests, seed, retry and replay controls;
- booted distribution, kernel, architecture, CPU/NUMA topology, governor,
  cgroup/resource limits, cache state, load policy, and network mode;
- complete terminal run manifest and journal, partial evidence, unavailable
  reasons, exclusions, contamination, and cleanup outcome.

Use `asb plan` before every launch. Keep cold and warm experiments separate,
declare bounds before measurement, and compare only runs whose structured
`compare` output says they are comparable. `report` validates terminal
journal evidence; it does not turn missing or censored observations into
success.

## Repeatable offline check

```sh
cargo test --locked -p asb-cli --test guide_examples -- --nocapture
```

This test performs independent runs, compares their validated definitions,
reports their durable evidence, and exercises a bounded sweep. It also verifies
that the command/workload support inventory equals live `doctor` output and
that unsupported commands fail.

Replay results measure a controlled agent/runtime path, not fresh model quality.
Emulation proves portability only. Native distribution/kernel claims require a
genuinely booted host. See [methodology](METHODOLOGY.md) and
[platform evidence](PLATFORMS.md).
