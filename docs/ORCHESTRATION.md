# Central orchestration authority

## Purpose

ASB has several lower-level capabilities—configuration, provider profiles,
credential enrollment, agent adapters, workloads, replay, sandboxing, leases,
and persistence. None of those capabilities may independently authorize a
benchmark. The `asb-orchestrator` service is the single runtime-owned authority
that turns a validated request into a bounded run and owns it until the final
outcome or reconciliation.

The service is required for both local and production paths. Local deterministic
mock and strict replay are the normal development modes. Live-provider mode is
explicitly optional and remains fail-closed unless the same authority chain is
complete. A real provider connection is never needed in CI.

## Ownership matrix

| Concern | Owner | Caller may supply |
| --- | --- | --- |
| Frontend transport and peer admission | `asb-control` | request bytes, idempotency key |
| Request validation and policy digest | `asb-orchestrator` | stable IDs, mode, bounded limits |
| Provider/model/workload catalog | catalog services via orchestrator | selected stable identifiers |
| Credential enrollment and secret channel | `asb-control` issues an opaque enrollment record; `asb-runtime` validates and mints the opaque credential capability | credential reference, never secret bytes |
| Provider target and egress policy | `asb-orchestrator` selects a catalog target; `asb-runtime` is the sole enforcement point | provider/model IDs only, no endpoint or socket path |
| Namespace, relay, sandbox and lease | `asb-runtime` invoked by orchestrator | no authority objects |
| Agent/workload lifecycle | `asb-orchestrator` through typed adapters | approved adapter identity |
| Journal, manifests and recovery | `asb-store` under orchestrator fencing | no private paths or raw output |
| Terminal UX and summaries | `asb-cli` / future frontends | opaque handles and bounded pages |

## Request and lifecycle

The public request is declarative and credential-free. It contains a stable
agent selection, provider profile and model IDs, workload/scorer revisions,
execution mode (`local-mock`, `strict-replay`, or explicitly admitted `live`),
resource/deadline limits, and an idempotency key. Unknown fields, unsupported
combinations, path traversal, unbounded limits, and stale catalog revisions are
rejected before any external effect.

The service owns this closed lifecycle:

```text
Admitted -> Prepared -> Running -> Collecting -> Completed | Failed | Cancelled
   |          |          |             |
   +----------+----------+-------------+--> NeedsReconciliation
                                              |
                                  Recovered -> Retry-authorized | Failed
```

Intent is journaled before process, lease, relay, credential, or provider
effects. Every attempt receives a causally fenced identity. A repeated
idempotency key returns the existing handle; a stale handle cannot mutate a
newer attempt. Recovery inspects process groups, cgroups, leases, relays and
artifacts before deciding whether cleanup or retry is safe. The only transitions
out of `NeedsReconciliation` are:

| Crash/race observation | Required action | Retry permitted |
| --- | --- | --- |
| Intent exists, no external effect | mark failed-before-effect | yes, with a new attempt ID |
| Child/lease/relay exists without terminal outcome | fence handle, terminate tree, release lease, remove relay, seal evidence | only after cleanup confirmation |
| Terminal outcome exists, cleanup uncertain | retain `NeedsReconciliation`, inspect and clean | no duplicate execution |
| Cancellation races with completion | compare causal event fence and keep the first terminal event | never repeat the attempt |

Timeout is a cancellation request followed by the same cleanup protocol. A retry
requires a new idempotency key or an explicit service-issued retry token; a
frontend cannot replay a stale handle.

## Authority acquisition

Only the orchestrator may call the runtime acquisition boundary. For each
attempt it obtains an opaque mode-specific capability:

- `LocalMockCapability`: loopback/mock backend, network denied.
- `ReplayCapability`: trusted cassette identity, relay and sandbox, network
  denied.
- `LiveProviderCapability`: enrolled credential reference, concrete allowlisted
  target, runtime-observed child namespace and relay, attested sandbox/backend,
  benchmark lease, launch token, and one live capability for that attempt.

The lower layers return opaque handles and bounded events. No caller receives
credential bytes or can replace a path, endpoint, lease, namespace, backend,
token, or relay after admission. Cancellation and timeout are service requests;
the attempt session owns relay, lease, child process tree, and credential
cleanup until terminal confirmation.

The versioned request, response, and event contract is checked in at
[`docs/orchestration-schema-v1.json`](orchestration-schema-v1.json). It fixes
field names, closed objects, identifier patterns, mode vocabulary, idempotency
limits, and numeric transport ceilings. AR-1452 adds Rust round-trip and hostile
fixtures for the same immutable contract before the service is enabled. The
design-level positive and unknown-field vectors are exercised by
`tests/quality/test_orchestration_schema.py`.

## Failure and security rules

- Local and replay modes preserve `NetworkPolicy::Deny`; direct and alternate
  egress are denied.
- Live mode is rejected before effects when any authority component is absent,
  stale, copied, mismatched, expired, or revoked.
- Provider reachability, native-host availability, and external API keys are
  optional supplementary evidence, never CI or local qualification gates.
- Native, container, and VM backends implement one bounded capability contract;
  each reports backend kind, architecture, kernel/libc feature flags, image or
  VM digest, tool digests, cgroup/namespace capabilities, and an attestation
  digest. The orchestrator admits only the requested capability set and records
  whether evidence is simulated, container-tested, VM-tested, or native-tested;
  it does not require a particular host distribution or architecture.
- Events and manifests contain identities, digests, statuses, and bounded
  metrics only. Each event is at most 64 KiB, each retained stream is subject to
  the runtime's 64 MiB ceiling, and event/artifact counts are bounded by the
  negotiated run limits. Schemas reject credential fields, prompt/transcript
  fields, raw provider bodies, private host paths, and unknown fields; redaction
  is performed before journaling rather than after publication.

## Migration

1. Freeze this ownership matrix and versioned request/event schemas (AR-1451).
   `ReplayLaunchFactory::issue` and `LiveLaunchFactory::acquire` become
   runtime-private implementation details; `LiveProviderRuntimeService` is
   consumed only by the orchestrator's runtime adapter. Existing CLI
   `run/sweep` injection seams are marked compatibility-only and are removed
   after the service path is qualified.
2. Implement `asb-orchestrator` and mode-specific runtime acquisition behind
   opaque handles; local mock/replay are first (AR-1452).
3. Route CLI and control dispatch through the service and retire direct
   injection-only paths (AR-1453). `asb-control` remains the transport; it
   cannot become a second run authority. The compatibility period is exactly
   one released protocol minor: old injection methods return a typed
   `CompatibilityOnly` error after the new service path is available, and are
   removed in the next major protocol version. `run`, `sweep`, `status`,
   `cancel`, `retry`, and result-page operations all map to the v1 operations
   in the checked-in schema.
4. Requalify first-customer install, local benchmark, replay, cancellation,
   recovery, cleanup, and optional live admission on disposable runners.
