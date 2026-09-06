# Agent Systems Benchmark (ASB)

ASB is a Linux terminal framework for measuring how AI coding agents scale:
how many concurrent sessions a system can sustain while task quality, latency
and resource consumption remain within declared bounds.

**Status: project bootstrap.** The CLI supports help/version and the core provides
checked concurrency and evidence-assessment primitives. Agent execution, metrics,
workloads and replay are planned in the public coordination repository.

- Implementation: Rust, safe code, explicit errors and versioned contracts.
- Target architectures: native x86_64 and aarch64 (arm64).
- Initial agent targets: OpenCode, OpenDesk CLI, aider and Codex.
- License: MIT, matching the CSB project's source license.
- [Development plan](docs/PLAN.md)
- [Architecture and extension API](docs/ARCHITECTURE.md)
- [Measurement methodology](docs/METHODOLOGY.md)
- [Workload catalogue](docs/WORKLOADS.md)
- [Replay research](docs/REPLAY_RESEARCH.md)
- [Quality and support matrix](docs/QUALITY.md)
- [Related benchmark systems](docs/RELATED_WORK.md)
- [Formal assurance roadmap](docs/FORMAL_ASSURANCE.md)
- [Worker process](docs/DEVELOPMENT.md)
- [Coordination tasks](https://github.com/martin-beck/agent-systems-benchmark-state)

## Build the bootstrap

Install the toolchain pinned in rust-toolchain.toml, then run:

```sh
cargo build --locked --workspace
cargo test --locked --workspace
cargo run --locked -p asb-cli -- --help
```

Future commands include doctor, plan, run, sweep, record, replay, compare and report.
They are design targets, not currently supported commands. No paid API call or
workload download is required by bootstrap tests.
