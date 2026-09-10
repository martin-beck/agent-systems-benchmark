# Agent Systems Benchmark (ASB)

ASB is a Linux terminal framework for measuring how AI coding agents scale:
how many concurrent sessions a system can sustain while task quality, latency
and resource consumption remain within declared bounds.

**Status: active development.** The CLI implements `doctor`, `capabilities`, `plan`, `run`,
`sweep`, `compare`, `report` and the local `serve` control endpoint. `run` and
`sweep` execute the original in-tree workloads through a digest-pinned
`batch-stdio-v1` executable, then persist bounded run evidence for reporting and
comparison. Provider recording/replay and live-provider selection are not CLI
commands yet; unsupported and unqualified platform combinations remain visible
in the public coordination repository.

The `asb-agents` library exposes a versioned `AllAgentsProviderSelection`
configuration boundary for applying one pinned OpenAI or verified Ollama profile
to a complete chosen agent set. It returns a complete canonical plan only when
every selected adapter preserves the same credential-free profile identity.
Empty, duplicate, mixed-provider, stale-version, lossy and unsupported selections
fail as a whole; per-agent overrides are deliberately a separate choice.

- Implementation: Rust, safe code, explicit errors and versioned contracts.
- Target architectures: required native x86_64 and pinned QEMU-emulated AArch64;
  native ARM64 is optional future qualification.
- Initial agent targets: OpenCode, OpenDesk CLI, aider and Codex.
- License: MIT, matching the CSB project's source license.
- [Development plan](docs/PLAN.md)
- [Native ARM64 and emulated AArch64 policy](docs/NATIVE_AARCH64_POLICY.md)
- [Architecture and extension API](docs/ARCHITECTURE.md)
- [Measurement methodology](docs/METHODOLOGY.md)
- [Workload catalogue](docs/WORKLOADS.md)
- [Measurement catalog](docs/MEASUREMENT_CATALOG.md)
- [Replay research](docs/REPLAY_RESEARCH.md)
- [Quality and support matrix](docs/QUALITY.md)
- [Pinned platform manifests](docs/PLATFORMS.md)
- [Related benchmark systems](docs/RELATED_WORK.md)
- [Formal assurance roadmap](docs/FORMAL_ASSURANCE.md)
- [Worker process](docs/DEVELOPMENT.md)
- [Offline quickstart](docs/QUICKSTART.md)
- [Agent and workload extensions](docs/EXTENSIONS.md)
- [Reproducibility guide](docs/REPRODUCIBILITY.md)
- [Coordination tasks](https://github.com/martin-beck/agent-systems-benchmark-state)

## Build and inspect the CLI

Install the toolchain pinned in rust-toolchain.toml, then run:

```sh
cargo build --locked --workspace
cargo test --locked --workspace
cargo run --locked -p asb-cli -- --help
cargo run --locked -p asb-cli -- doctor
cargo run --locked -p asb-cli -- capabilities --format json
```

The implemented command forms are:

```text
asb doctor
asb capabilities --format json
asb plan EXPERIMENT.toml
asb run EXPERIMENT.toml
asb sweep EXPERIMENT.toml
asb compare RUN...
asb report RUN...
asb serve CONTROL.toml
```

`plan` validates without launching. `run` executes one configured capacity point;
`sweep` executes the bounded range in the plan. `compare` requires at least two
persisted run directories and `report` requires at least one. Structured results
and errors are JSON on stdout, progress is on stderr, and invalid usage returns a
nonzero status. See `asb --help` for the authoritative command list. There is no
`asb record` or `asb replay` command in the current CLI. No paid API call or
workload download is required by repository tests.

`asb capabilities --format json` is a bounded, deterministic, side-effect-free
description of the closed protocol implemented for independent frontends. Its
checked schema is
[`crates/asb-cli/schema/v1/capabilities.schema.json`](crates/asb-cli/schema/v1/capabilities.schema.json).
The response contains only the protocol and ASB versions plus operation booleans;
it does not inspect or expose host, user, provider, credential, or socket data.

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
