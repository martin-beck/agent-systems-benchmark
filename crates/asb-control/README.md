# ASB frontend control API

`asb-control` is the bounded local boundary between the authoritative ASB
runner and independent frontends such as `asb-tui`. Version 1 uses JSON-RPC
2.0 bodies with a four-byte big-endian length prefix over a Linux Unix-domain
socket. The socket is created with mode 0600 inside a same-user directory with
no group or other access, and every accepted peer is checked with
`SO_PEERCRED`. The frontend performs the same kernel credential check on the
connected server before sending negotiation or request content; peer identity
values are inspectable but cannot be constructed through the public API.
Authentication error text is fixed and does not disclose host user IDs.

The first complete JSON-RPC envelope negotiates an exactly offered supported
version and intersects frame, deadline, page, and in-flight limits. Calls cover
capabilities, the content-addressed measurement catalog, settings
validation, durable plan creation, launch, status, causal cancellation, recent
history, exact repeat, analysis, resumable public events, and privacy-safe
artifact metadata. There is no implicit TCP listener. Remote transports are a
separate explicitly enabled boundary.

The endpoint implements exact serialized versions `1.0` and `1.2`. Existing
clients continue to negotiate immutable `1.0`; independent frontends offer
`1.2` to obtain `measurement_catalog`. The additive `1.1` evidence types do not
define a separately selectable wire version.

Runner journals remain authoritative. Mutating calls carry idempotency keys;
the runner must journal the canonical request and terminal public result before
indexing it as committed. Reusing a key with another request fails closed.
Disconnecting or crashing a frontend drops only connection-local admission
state and never cancels or repeats a run. Reconnect cursors either produce a
contiguous bounded page or an explicit stale/future error.

`ControlServer` is the runner-side owner-only endpoint and `ControlClient` is
the independent frontend library. The backend trait receives the one absolute
monotonic deadline that began before frame ingress and owns durable journal
fencing. The endpoint applies the diminishing budget to reads and writes and
never emits a late backend success. Backends are trusted runner components and
must cooperatively stop before a commit at expiry or durably mark the effect
`needs_reconciliation`; a non-cooperative backend may occupy one bounded
connection worker until it returns, but is never detached and cannot mutate
after a response.

Ordinary successful responses are a closed typed allowlist with validated
identities, digests, pagination, and method/result association. Fixed error
messages are bounded. Responses contain public summaries and content digests,
never host paths or artifact contents. Sensitive artifact content requires a
separately authorized interface. The checked-in schemas and public fixtures are
conformance material, not authorization policy.

Decode `ControlResponse` directly from borrowed raw JSON with `read_frame`,
`serde_json::from_slice`, or `serde_json::from_str`. Conversion through
`serde_json::Value` is intentionally unsupported because normalization erases
the exact catalog bytes needed for duplicate-key and wire-size enforcement.
