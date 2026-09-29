# Development credential enrollment

ASB's development setup uses the versioned `asb-config` contract exposed by
`DevelopmentCredentialStore`. It is deliberately a provider-free fixture
boundary, not production credential security. The request and response types
reject unknown fields, carry a schema version, and expose only stable provider,
model, authentication-route, generation, status, warning, and digest values.

The supported operations are `enroll`, `test`, `rotate`, `reset`, and `status`.
Mutations are fenced by the generation returned in the previous status and are
retry-safe through an idempotency key. Reset advances the generation, so an
older test or rotation cannot change the new state. The store can be serialized
and restored after a restart; its public JSON contains no secret, private key,
endpoint, process path, prompt, or provider response.

When authentication, signature validation, or key management is absent, the
development path creates a deterministic generated identity and returns a
visible `development-only` warning. This warning does not block setup, local
mock capture, strict replay, or comparison. The deterministic identity and
self-signature are fixture digests only and must not be presented as a
production trust decision.

`asb-development-mock` / `fixture-model-v1` is the checked-in local provider
pair. It performs no network I/O. Other provider/model pairs are accepted only
when the bounded compatibility matrix says the selected authentication route
is valid; actual provider reachability and production credential hardening are
owned by later work.

## Local provider/capture fixture

`asb_agents::DevelopmentProviderFixture` connects an enrolled development
status to one provider/model selection and applies that selection atomically to
the selected agent set. `default_for_all` creates the setup default, while
`apply_to_all_defaults` replaces the complete set without permitting a mixed
provider configuration. Every capture carries a generation-bound public
receipt and an explicit fixture seed; stale generations, provider/model
mismatches, malformed receipts, and idempotency conflicts fail closed.
The receipt preserves the AR-1499 enrollment self-signature digest and also
contains a separate seed-bound fixture signature; neither value is a
production trust assertion.

The fixture produces deterministic loopback-only mock exchanges, seals them
through the normal redaction and cassette integrity boundary, and exposes the
existing strict replay service. Replay has no provider fallback or network
path. Serialization, cancellation, interrupted capture recovery, and complete
selected/all-agent comparison readiness are covered by Rust tests. The fixture
warnings about missing authentication, signature validation, and key
management are intentionally visible but never block this development path.
Restart state is bounded before JSON parsing and is integrity-checked on
restore. It is not production provider evidence; production credential and
signing hardening remains the responsibility of AR-1501.

The control-runtime `auth_helper_invoke` mutation commits its public
`AuthStatus` projection together with the digest-only enrollment record. The
runner binds provider, endpoint digest, locator digest, generation, and status
before accepting the catalog commit; a mismatched projection remains
`needs_reconciliation` and is never silently retried. Repeating the same
idempotency key returns the committed projection after restart. This is a
provider-free runtime consistency guarantee, not production authorization or
provider-reachability evidence.
