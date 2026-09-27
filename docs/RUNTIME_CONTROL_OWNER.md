# Runtime/control process-owner contract

The runtime/control process owner is the sole owner of the authenticated
control session, certificate-chain store, private authority resolver, opaque
dispatch-source minting, cancellation, and teardown. The CLI receives only a
one-shot opaque source through the AR-1480 composition seam.

The stable Rust contract is `asb_runtime::control_owner_contract` schema `1`.
Its public projection contains only:

| Field | Meaning |
| --- | --- |
| `schema_version` | Closed contract version; unknown versions fail closed. |
| `owner_id` | Bounded owner identity, never a credential or authority token. |
| `generation` | Positive enrollment generation fence. |
| `chain_sha256` | Digest of the authenticated certificate chain. |
| `receipt_nonce_sha256` | Digest of the one-shot receipt nonce. |
| `state` | `prepared`, `enrolled`, `issued`, or a terminal failure state. |

Lifecycle transitions are owner-controlled:

`prepared -> enrolled -> issued -> (cancelled|revoked|expired|disconnected|torn_down)`

Each terminal state rejects further transitions. Unknown JSON fields, invalid
digests, zero generations, malformed projections, replayed receipts, stale or
mismatched chains, and unavailable control state fail closed. The projection
never contains credentials, endpoints, policy, lease/relay roots, tools,
namespace paths, or launch tokens.

Qualification uses deterministic local/mock and strict-replay fixtures only;
live providers and network access are optional and never completion gates.

## Provider-free process-owner qualification

`LocalMockRuntimeControlOwner` is the bounded local qualification owner. It
creates an ephemeral mock backend from the validated contract, enrolls before
issuing an attempt, and revokes that backend during teardown. Issuance before
enrollment and issuance after teardown are rejected. The mock backend is test
evidence only: it cannot mint production authority, accept caller-supplied
credentials, or establish a live-provider connection.

The ordinary provider-free CLI qualification entry point is
`run_with_runtime_control_local_mock_owner`. It accepts only the runtime-owned
owner object, enrolls before `run` or `sweep`, and tears the owner down before
returning. Reuse after teardown and malformed entry shapes fail before result
roots or execution state are created. Production/live dispatch continues to
use the separate AR-1480 opaque source and is unavailable without an
authenticated runtime source.
