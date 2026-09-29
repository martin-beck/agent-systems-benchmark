# Platform-owned authority provider

The runtime/control boundary owns the private authority needed by a live
provider launch. `RuntimeControlBootstrap` passes a provider only the
secret-free, authenticated AR-1505 binding: control session, restart and
cancellation bindings, generation and expiry, provider and endpoint identity,
credential reference, namespace, lease and relay root claims, and the pinned
tool-bundle digest.

The provider returns private roots, namespace identity, tool pins, egress
policy and allowlist, credential capability reference, enrollment material,
and its runtime-owned persistence location through the crate-private provider
contract. The bootstrap recomputes the material binding before creating the
resolver. A missing, stale, expired, revoked, restarted, cancelled, tampered,
or alternate-egress result is rejected without exposing private values.

## First-customer platform handoff

The platform deployment must supply an authenticated implementation of the
runtime provider contract and independently verify its platform root, relay
root, lease root, namespace, tool bundle, policy, credential reference, and
enrollment/receipt source. The implementation must bind all of those values to
the AR-1505 session and generation, keep them inside runtime/control, and
revoke them on cancellation, teardown, expiry, or restart.

Qualification uses only deterministic local, mock, or replay fixtures. It does
not establish live provider reachability, production credential validity,
first-customer performance, or native platform support. No CLI configuration,
socket, environment `PATH`, fixed host path, or `asb-tui` workflow may provide
authority material.
