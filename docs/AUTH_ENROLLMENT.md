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

## Development CLI setup

The development CLI exposes a deliberately ephemeral OpenRouter setup path:

```text
printf '%s\\n' "$OPENROUTER_API_KEY" | asb auth setup --provider openrouter --api-key-stdin --json
```

The command validates the key at the resolver boundary, immediately drops it,
and returns only a typed `configured` flag and development-only warning. It does
not persist the key, include it in JSON, logs, argv, evidence, or the provider
selection digest. Without `--api-key-stdin`, it checks the existing
`OPENROUTER_API_KEY` environment channel and returns a warning-only unavailable
state when no key is present. This command does not contact OpenRouter; live
provider reachability remains an explicit runtime operation.

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

The negotiated `control v1.11` `runtime_bootstrap` operation is the
platform-owned handoff for a live runtime. The request contains only the
provider generation, a session digest derived from the kernel-authenticated
control connection, a fresh nonce, and a restart binding. The response adds a
bounded expiry and opaque cancellation binding to the existing chain and
receipt metadata. `runtime_bootstrap_cancel` revokes that exact session and
generation; a later bootstrap for the revoked session/restart binding is
rejected. A caller-supplied session digest is rejected by the backend unless
it matches the authenticated peer identity, so the digest is not an authority
input.

The runtime receipt response carries bounded public
`AuthenticatedChainEnrollmentV1` metadata. Control binds its canonical chain
digest to the receipt before sending it over the owner-authenticated control
socket. Before dispatch, an owned platform adapter is constructed with the
authenticated control-session, runtime namespace, relay-root, lease-root,
credential-reference and expiry binding. The adapter is sealed from frontend
implementations and is the only source accepted by
`RuntimeCertificateChainStore::enroll_from_authority`; its private authority
material never enters the request or durable record. Enrollment rejects a
caller-built chain, a source bound to a different request digest, and a chain
whose generation, endpoint, subject or canonical digest differs from the
control enrollment.

The store validates the receipt against that source-issued chain and all
provider, credential, lease, relay, generation and expiry fields before live
dispatch. Cancellation revokes the active generation, and a fresh runtime
starts with an empty store; both states fail closed. A replayed, stale,
expired, revoked or mismatched response is rejected without egress.

Production platform handoff requirements are therefore: (1) derive the
binding digest from the authenticated session and runtime-owned resource
roots, (2) retain the signing/trust material inside the platform adapter,
(3) issue only the selected generation and endpoint, and (4) revoke the
generation before cancellation or teardown completes. The local adapter tests
prove this contract with deterministic metadata only; they are not production
provider authority or reachability evidence.
