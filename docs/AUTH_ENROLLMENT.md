# Provider authentication enrollment

`asb-agents::auth` owns the credential-free enrollment record used by provider
connections. A record contains only a stable connection identifier, credential
source, locator digest, generation and public probe status. It never contains a
key, locator, provider response, or filesystem path.

Applications provide a qualified [`SecretBackend`](../crates/asb-agents/src/auth.rs)
implementation for their system secret store or one-shot descriptor/helper.
`enroll_api_key` validates the bounded input and passes it once to that backend;
the enrollment record is safe to persist with `to_json` and restore with
`from_json`. Backends must keep values private and return only
`connected`, `rejected`, `expired` or `unavailable` probe outcomes.

Every probe carries the enrollment generation. A replacement increments the
generation and resets status to `untested`, so a delayed probe cannot restore a
stale credential. Revocation is terminal for the record. No environment,
argument, control-frame or public-evidence fallback is performed.

## Runner-owned control handoff

The setup wizard must not execute a helper path itself. The AR-1324 control
operation will accept only a typed, credential-free provider profile and an
idempotency/generation binding. The runner resolves the provider's private
allowlisted helper registration, invokes the sealed `HelperCredentialResolver`
with bounded output and timeout, discards the resolved secret inside the
backend boundary, and returns only the provider identity, endpoint digest,
locator digest and a closed success/failure/cancel outcome. Missing helper
registration, stale generations, path substitution, ambient environment,
unexpected stdout/stderr and raw-key-shaped output all fail closed.

The negotiated `control v1.10` `auth_helper_invoke` operation implements this
handoff. The request carries only a provider identifier, a complete
credential-free `ProviderProfileV1`, and an idempotency key. The runner reads a
private allowlisted helper registration (`ASB_AUTH_HELPER_EXECUTABLE`, its
content digest, and a logical locator), opens the executable with no-follow
semantics, resolves it through `CredentialBackend::Helper`, and drops the
resolved credential before returning a typed public auth receipt. Missing or
invalid registration, profile/reference mismatch, helper timeout, malformed
output, and deadline expiry are rejected without exposing helper paths or
secret material.

## Runtime certificate chain boundary

The control runtime accepts a certificate chain only when the enrollment authority
has pinned the DER trust anchor and generation. The CertificateAuthorityV1 issue_der
operation validates the leaf and ordered intermediates with the offline
rustls/webpki verifier; it does not fetch roots, consult a system trust store, or
fall back to digest-only metadata. The leaf digest must match the identity record
and the pairing fingerprint must match the leaf subject identity. Missing anchors,
malformed DER, issuer/anchor mismatches, expired or not-yet-valid certificates,
unsupported roles, stale generations, and unknown fields fail closed. Only bounded
identity digests and authorization outcomes are suitable for durable public evidence;
certificate and private-key bytes must remain in the caller's protected memory/store.
