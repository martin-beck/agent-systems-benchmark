# Provider-aware launches

`provider-plan` is deliberately a side-effect-free selection step. A saved selection becomes
authoritative only when `run` or `sweep` constructs a `ProviderLaunchV1` binding immediately
before creating the durable run. The binding content-addresses the catalog, complete selection,
provider profile, exact agent/adapter route, model/settings, runtime and executable identities,
credential resolver reference, workload/run/attempt identities, and bounded process policy.

The binding is constructor-controlled: the adapter projection must exactly equal every provider,
model, API-mode, settings, and resolver field in the requested launch. Any stale, conflicting,
unknown, or tampered identity fails before run creation or process spawn. The canonical launch
digest is stored in the run definition and terminal report, and is exported to the adapter only as
non-secret `ASB_PROVIDER_*` environment metadata. Credential values never enter argv, ordinary
environment, manifests, artifacts, logs, or errors; the approved resolver boundary remains the
only secret channel.

Every selected agent must have an exact projection. A generic wrapper cannot claim a different
provider by changing its defaults: the launch digest and profile identity are fixed by the
constructor-controlled projection and are revalidated immediately before spawn. Replay remains
offline and must use its existing exact cassette/network-denial contract.

The current experiment plan carries one verified executable identity. Until runtime-bundle
manifests are included in the plan schema, that executable content address is used as the
fail-closed runtime identity; it must match again at snapshot/spawn time. This is an explicit
evidence boundary, not a claim of native third-party provider-service qualification.
