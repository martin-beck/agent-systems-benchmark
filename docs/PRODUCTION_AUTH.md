# Production authentication boundary

`asb-agents::production_auth` is the production credential contract. It is
separate from the development-only generated identity in
`asb-agents::auth`/`asb-agents::development_fixture`.

The contract stores only a provider binding, endpoint digest, secure-store
reference, generation, and bounded lifecycle status. Secret bytes are accepted
once by `SecureSecretStore::put`; they are not returned, serialized, logged, or
placed in control frames. Deployments must provide a reviewed adapter backed by
an OS keychain/secret service, protected descriptor, or pinned helper. The
default `FailClosedSecureSecretStore` rejects enrollment, so a missing keychain
cannot silently fall back to environment variables or generated development
credentials.

Provider authentication is explicit: OpenAI-compatible routes use bearer
authorization, Anthropic uses `x-api-key`, Gemini uses `x-goog-api-key`, and
Ollama must explicitly choose bearer or no-auth. Endpoint identity is hashed
and checked on every binding and verification request.

Remote verification is an injected, bounded `RemoteVerifier`. The result must
match the current enrollment generation and endpoint digest and must report
successful authentication before an enrollment becomes `active`. Delayed,
cross-endpoint, malformed, or rejected results fail closed. Tests use a local
deterministic verifier; live provider access is never a development gate.

Rotation is compare-and-swap fenced. The replacement is staged before the old
reference is revoked; if revocation fails the replacement is rolled back and
metadata does not advance. Revocation is terminal. `AuditLogV1` retains at
most 128 credential-free events and records only stable identifiers and
digests, never secrets, endpoint strings, response bodies, or host paths.

This AR establishes the reviewed contract and test evidence. A deployment must
still provide an independent security review and platform-specific keychain
adapter qualification before claiming production customer readiness.
