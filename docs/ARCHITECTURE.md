# Architecture and extension contracts

## Language decision

Use Rust 2024 with a pinned toolchain for every new product component. Forbid
unsafe code in first-party crates, deny compiler warnings, use checked units and
exhaustive enums, and represent invalid configuration as errors. OS integration
must use audited safe wrappers; exceptions require a scoped architecture decision.
Rust prevents many memory and data-race errors; it does not prove deadlock freedom,
correct statistics, external process safety or kernel correctness.

Retain the reused strictly typed Python coordinator outside the product runtime.
Do not rewrite its proven locking and lease behavior merely for language uniformity.
A future Rust coordinator requires differential tests against its existing model.

## Boundaries

| Planned crate | Ownership |
| --- | --- |
| asb-core | IDs, units, validated plans, outcomes and pure state machines |
| asb-protocol | Versioned manifests, JSON Schema, JSON-RPC envelopes and events |
| asb-runtime | Process groups, deadlines, cancellation, sandbox and resource leases |
| asb-agents | Separate OpenCode, OpenDesk, aider and Codex adapters |
| asb-workloads | Dataset acquisition, preparation, independent grading and cleanup |
| asb-metrics | procfs/cgroup collectors, optional perf/eBPF integrations |
| asb-replay | Provider-boundary recording, matching and paced streaming |
| asb-analysis | Experimental design, statistics, SLO assessment and comparisons |
| asb-store | Atomic manifests, event journal, artifact hashes and recovery |
| asb-control | Bounded local frontend protocol, admission, cursors and Unix socket |
| asb-cli | Terminal UX and machine-readable CLI output |
| asb-csb | Optional CSB subprocess integration and result mapping |
| asb-bundle | Signed runtime-bundle manifests and offline content/SBOM/license verification |

Only asb-core and asb-cli exist at bootstrap. Add crates when their AR starts.
Core must not depend on process, network, terminal or GitHub implementations.

The runtime bundle contract and its trust boundary are documented in
[runtime bundle verification](RUNTIME_BUNDLES.md).

## Frontend control boundary

Independent frontends use the versioned `asb-control` API rather than linking
to CLI internals. Local v1 carries JSON-RPC 2.0 bodies in four-byte
big-endian-length frames over an owner-only Unix socket. The socket lives in a
same-user private directory, uses mode 0600, and authenticates each peer using
Linux `SO_PEERCRED`. It never opens TCP implicitly; explicitly enabled remote
transport is a separate security boundary.

The runner journal is authoritative for plans, attempts, cancellation, history,
and events. Mutations are journal-backed and idempotent, attempts are causally
fenced, pages and event retention are bounded, and stale or future cursors fail
explicitly. A frontend connection owns only its request admission state, so
disconnect or crash cannot cancel or repeat a run. Public summaries never
contain host paths or sensitive artifact contents. See
[Frontend control API](FRONTEND_CONTROL_API.md).

## Extension API v1 design

Rust traits serve built-in extensions. Independently executable extensions use
JSON-RPC 2.0 over newline-delimited JSON on stdio, with negotiated protocol major
and minor versions. Never expose Rust's unstable dynamic-library ABI as a plugin API.
Require bounded frame sizes, deadlines, backpressure, request IDs, typed errors,
cancellation and capability negotiation. Logs use stderr and cannot corrupt stdout.

Agent methods: describe, probe, prepare, start, cancel, collect and cleanup.
Events: ready, request_started, first_response, tool_started, tool_finished,
usage, completed and failed. Session and attempt identities accompany every event.
Unknown capability means unavailable, not supported. Pin executable/package hashes.

Workload methods: describe, acquire, prepare, prompt, evaluate and cleanup.
Manifest fields include ID, version, license, source revision, content hash,
architecture/OS requirements, fixture size, timeout, resource budget, scoring version
and allowed network destinations. The first in-tree workloads must use this API.

Collector methods: probe, start, sample and stop. Every metric declares unit,
aggregation, measurement scope, source, resolution and unavailable reason.
Execution backends: native subprocess, isolated container, VM and optional CSB.
Native execution is for trusted fixtures; untrusted agent-generated code uses isolation.

## Durable execution

Planned -> Prepared -> Running -> Collecting -> Completed, Failed or Cancelled.
Persist intent before external effects and outcomes afterward. An interrupted
Running state becomes NeedsReconciliation: inspect processes, cgroups and artifacts
before repeating work. Stable attempt IDs reject stale events. Cancel the whole
process tree, reap children and publish partial evidence on timeout.

Separate immutable run manifests, bounded event journals and artifact storage.
Record toolchain, agent/model identity, workload revision, machine/kernel metadata,
image digests, collector settings, random seeds and all measurement exclusions.

## CSB decision

CSB's bm-runner uses benchkit, native/container execution units and a MonitorFactory.
Its executor controls concurrency, duration, barriers, monitor startup and cleanup.
It already accepts external applications separately from generated C benchmarks.
The inspected checkout was f97bfdf2a7cca7c4f088584f9ce34d3fae1cc80e.

Use an optional subprocess adapter with a pinned compatible CSB revision and an
explicit artifact mapping. ASB owns agent lifecycle and grading; only one framework
owns concurrency and monitoring in a run. Avoid nested schedulers and double counting.
No default dependency on Python, benchkit, bm-generator, syzkaller or CSB submodules
in the Rust runtime. AR-0601 must demonstrate cancellation, timestamp alignment and
round-trip result fidelity before calling the integration supported.

Reference: [CSB](https://github.com/martin-beck/CSB).
