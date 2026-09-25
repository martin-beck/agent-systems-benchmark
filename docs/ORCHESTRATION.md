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
| Credential enrollment and secret channel | runtime/control authority | credential reference, never secret bytes |
| Provider target and egress policy | orchestrator + runtime | no endpoint or socket path |
| Namespace, relay, sandbox and lease | `asb-runtime` invoked by orchestrator | no authority objects |
| Agent/workload lifecycle | orchestrator through typed adapters | approved adapter identity |
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
Admitted
  -> Prepared
  -> Running
  -> Collecting
  -> Completed | Failed | Cancelled
                 ^
                 |
        NeedsReconciliation (after interruption)
```

Intent is journaled before process, lease, relay, credential, or provider
effects. Every attempt receives a causally fenced identity. A repeated
idempotency key returns the existing handle; a stale handle cannot mutate a
newer attempt. Recovery inspects process groups, cgroups, leases, relays and
artifacts before deciding whether cleanup or retry is safe.

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

## Failure and security rules

- Local and replay modes preserve `NetworkPolicy::Deny`; direct and alternate
  egress are denied.
- Live mode is rejected before effects when any authority component is absent,
  stale, copied, mismatched, expired, or revoked.
- Provider reachability, native-host availability, and external API keys are
  optional supplementary evidence, never CI or local qualification gates.
- Native, container, and VM backends implement one bounded capability contract;
  the service does not require a particular host distribution or architecture.
- Events and manifests contain identities, digests, statuses, and bounded
  metrics only. They never contain credentials, prompts, transcripts, raw
  provider bodies, private host paths, or unbounded subprocess output.

## Migration

1. Freeze this ownership matrix and versioned request/event schemas (AR-1451).
2. Implement `asb-orchestrator` and mode-specific runtime acquisition behind
   opaque handles; local mock/replay are first (AR-1452).
3. Route CLI and control dispatch through the service and retire direct
   injection-only paths (AR-1453).
4. Requalify first-customer install, local benchmark, replay, cancellation,
   recovery, cleanup, and optional live admission on disposable runners.

