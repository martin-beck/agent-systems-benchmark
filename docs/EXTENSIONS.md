# Agent and workload extension guide

The ASB architecture defines two extension boundaries. Built-in extensions
implement safe Rust traits; the external-extension contract uses versioned
JSON-RPC 2.0 over newline-delimited JSON on standard I/O. The current CLI has
implemented only the built-in workload and digest-pinned `batch-stdio-v1`
agent path; general external-extension discovery and transport are not yet a
CLI feature. Rust dynamic libraries are not a supported plugin ABI.

## Agent lifecycle

Implement `describe`, `probe`, `prepare`, `start`, `cancel`, `collect`,
and `cleanup`. Emit bounded events carrying the exact session and attempt IDs:
`ready`, `request_started`, `first_response`, `tool_started`,
`tool_finished`, `usage`, `completed`, or `failed`.

The CLI currently executes only a digest-pinned `batch-stdio-v1` program:

- stdin is the workload prompt through a private unlinked descriptor;
- the current directory is the prepared workspace;
- arguments must be empty, so provider flags belong in a separately pinned
  wrapper;
- stdout and stderr, wall time, process tree, environment, and output size are
  bounded;
- unknown capabilities and malformed output fail closed.

Do not emit prompts, credentials, source contents, or private host paths as
public evidence. Test cancellation, timeout, truncation, executable replacement,
malformed events, cleanup uncertainty, and unavailable capabilities.

## Workload lifecycle

Implement `describe`, `acquire`, `prepare`, `prompt`, `evaluate`, and
`cleanup`. Bind the manifest to its ID, revision, content digest, license,
platform requirements, timeout and resource budget, plus the protected scorer
revision and digest. Agent-writable files must be disjoint from the grader and
expected-answer boundary.

Use the original workload suite as the executable reference:

```sh
cargo test --locked -p asb-workloads
cargo test --locked -p asb-cli --test guide_examples
```

The stable IDs are machine-checked in
[the guide contract](examples/guide-contract.json). External datasets require
separate code/data licenses, pinned acquisition and evaluator revisions, and
explicit platform evidence. A container or cross-build is not native evidence.

See [architecture](ARCHITECTURE.md), [workloads](WORKLOADS.md), and
[quality gates](QUALITY_GATES.md) for the complete contracts.
