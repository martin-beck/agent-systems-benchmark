# Beginner workflows

This is the shortest path through a complete Agent Systems Benchmark (ASB)
journey. The examples use one public workload, `original.bug-fix`, and keep
the same vocabulary in the CLI and TUI: catalog, settings, plan, launch,
follow, terminal state, report, compare, record, and replay.

## Choose a route

| Goal | CLI | TUI/control surface |
| --- | --- | --- |
| Build and validate a first plan | [CLI first run](cli-first-run.md) | [TUI first run](tui-first-run.md) |
| Select several agents for one provider | [Shared provider](multi-agent-provider.md) | Same page, `MultiAgentWizard` |
| Follow, finish, and compare runs | [CLI first run](cli-first-run.md#follow-finish-and-compare) | [TUI first run](tui-first-run.md#follow-finish-and-compare) |
| Capture and replay provider responses | [Record and replay](record-replay.md) | Same page, `RecordingWorkflow` |
| Recover from a failed or stale run | [Troubleshooting](troubleshooting.md) | [TUI recovery](tui-first-run.md#reconnect-and-recover) |

## Fastest verified CLI path

From a clean checkout with the pinned toolchain:

```sh
cargo build --locked --workspace
cargo run --locked -p asb-cli -- doctor
cargo test --locked -p asb-cli --test guide_examples -- --nocapture
```

The test creates a disposable, digest-pinned agent and proves `plan`, `run`,
`sweep`, `report`, and `compare` without credentials or provider traffic. For
a real plan, start with `asb plan`; it has no launch effect.

## Important boundaries

- `asb-tui` is a negotiated frontend library plus a startup shell. The current
  released binary prints `asb-tui requires a negotiated local frontend
  connection`; it does not promise a keyboard map or standalone rendering.
- The TUI pages therefore name the exact model steps and `ControlCall`s that a
  negotiated renderer must expose. They do not claim that a terminal key or a
  local transport exists when it does not.
- Live provider selection, recording, and replay are explicit. Credentials are
  represented only by a digest of a logical reference; never paste a secret in
  a command, plan, environment, or report.
- A replay result is a controlled offline execution, not fresh model-quality
  evidence. Compare only runs whose structured result says they are comparable.

## Advanced references

- [Offline quickstart](../QUICKSTART.md)
- [Provider-aware launches](../PROVIDER_LAUNCH.md)
- [Reproducibility](../REPRODUCIBILITY.md)
- [Quality gates](../QUALITY_GATES.md)
- [Frontend control API](../FRONTEND_CONTROL_API.md)
