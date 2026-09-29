# Authenticated process-owner material

`asb-runtime::process_owner_material` is the runtime-owned bridge from an
enrollment-backed owner lease to the opaque live-provider dispatch source.
The public contract is a secret-free projection only.  It cannot be used to
reconstruct roots, credentials, policy, tools, or launch authority.

An issuer is created only by the authenticated runtime/control composition
layer.  There is no public store or issuer constructor.  A caller supplies a
fresh nonce; the owner validates the enrollment binding, consumes the nonce,
and returns an opaque lease.  Expiry, restart, cancellation, remote revoke,
and teardown are checked again when a dispatch source is consumed.

Pinned executable material is checked at issuance and dispatch: the path must
be absolute, canonical, regular, executable, non-symlink, and byte-identical
to its lower-case SHA-256 pin.  Adapter identity and the complete tool bundle
are included in the authenticated contract.  Path, target, alternate-egress,
unknown-field, replay, and digest drift fail closed.

The dispatch bridge accepts only the exact sandbox launch input and adapter
digest carried by that lease.  It also rechecks the source's enrollment-bound
policy, target, and alternate-egress digests before consuming the lease, so a
caller cannot substitute a merely well-shaped executable or adapter.  The
ordinary provider-free CLI path exposes this composition as `asb run
EXPERIMENT.toml --local-mock` and `asb sweep EXPERIMENT.toml --local-mock`;
these commands exercise the same enrollment and teardown boundary without
claiming provider reachability.

This boundary qualifies provider-free control composition.  It does not claim
that a provider is reachable or that a live provider was contacted.
