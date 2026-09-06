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
- [Pinned platform manifests](docs/PLATFORMS.md)
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

## OpenDesk adapter boundary

The built-in OpenDesk adapter targets exactly `@bitclub.ai/opendesk-cli` 0.3.5
at upstream revision `f303069da72412dc90b3214d89da9c102282465f`. It verifies
both the Linux x86_64 wrapper and its Node.js 26.3.0 runtime by SHA-256 before
execution. Other versions, operating systems and architectures are unsupported.
The inspected source declares MulanPSL-2.0, while the npm tarball's package
metadata names that license but the tarball contains no license text. ASB does
not redistribute the artifact; this pin is compatibility evidence, not approval
to redistribute it.

OpenDesk's noninteractive interface places the prompt in a process argument and
its upstream client attempts an unconditional telemetry connection. Therefore
secret prompts are unsupported. ASB uses an isolated per-attempt home, sets a
closed HTTP(S) proxy, activates the pinned Node runtime's environment-proxy
dispatcher, and places only the selected provider host in `NO_PROXY`;
any provider host whose `NO_PROXY` scope would include OpenDesk's telemetry host
is rejected. These environment
controls are defense in depth, not a network sandbox. Strong network and
filesystem containment remains the responsibility of the ASB sandbox layer.
