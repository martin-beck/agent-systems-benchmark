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
