# asb-replay cassette contracts

`asb-replay` defines the immutable, versioned, privacy-reviewed response
cassette boundary. AR-0502 does not provide an HTTP server, replay cursor,
matching service, pacing, network interception, or a provider compatibility
claim. Those behaviors require later ARs and real-client conformance evidence.

## Version 1 invariants

The root integrity digest covers the v1 canonical JSON encoding of `contents`.
Request bodies and buffered/event payloads also carry canonical SHA-256 values
computed only after redaction. The decoder validates every digest before
returning a cassette and rejects unknown
fields, unknown schema or policy versions, malformed JSON, truncation, invalid
event ordering, forward causal references, duplicate response identities, and
configured size/count overruns.

V1 canonical JSON is UTF-8 with no insignificant whitespace. Object member names
are sorted recursively by their UTF-8 bytes; array order is retained. Strings,
numbers, booleans, and null use the exact scalar spelling emitted by the pinned
`serde_json 1.0.143` serializer. Implementations using another JSON stack must
match the checked-in byte vectors; V1 does not claim compatibility with a
different numeric or escaping canonicalization.

The cassette persists the complete sorted, non-secret header/query/body selector
configuration and its canonical SHA-256 identity, not only the policy version.
This lets audit and migration tooling distinguish materially different V1
redaction policies.

Requests store every response-affecting semantic field explicitly: method,
validated canonical origin-form path/query, headers, body, model, options, tools,
and prior-response identity. Network-path references, fragments, backslashes,
control characters, and parser-normalized request targets fail closed. Header
values containing control characters also fail closed.
Event boundaries and monotonic offsets are independent of transport chunk sizes.
A streamed response has exactly one terminal classification on its final event.

Default hard ceilings are 256 MiB per cassette, 16 MiB per request, 64 MiB per
buffered response, 16 MiB per event, 65,536 events, 16,384 interactions, and
1,024 headers. Callers may tighten but not raise these limits. Version 1 stores
uncompressed JSON; compressed input is unsupported, avoiding an unbounded
decompression boundary.

## Redaction

Redaction occurs in memory before sealing. Version 1 matches configured headers
case-insensitively, decoded query names, and explicit RFC 6901 body pointers.
Equal semantic strings across headers, query parameters, and JSON string fields
receive the same placeholder. Typed mapping keys keep distinct strings and
non-string JSON representations from collapsing. Placeholder-shaped input,
ambiguous duplicate headers, invalid
pointers, double-encoded query escapes, absent configured fields, and mapping or
selector exhaustion fail closed. The ephemeral reverse mapping is never part of
the cassette or an error message.

All cassette and selector hashes are unkeyed integrity checks. They detect
accidental corruption and inconsistent descriptors; they do not authenticate an
author, resist deliberate recomputation, or prove that an externally supplied
cassette contains no secrets. Only values produced through the in-process
`Redactor` wrapper are accepted by `seal_cassette`; external cassettes still
require provenance and privacy review.

Public fixtures contain only synthetic values. Raw captures, real prompts,
responses, credentials, private paths, and transcripts must never be committed.

## Migration

Before accepting any future migration, `verify_migration_references` requires
the source and destination to preserve session, attempt, interaction, response,
prior-response and tool-call identities, all causal edges, interaction/event
order, monotonic offsets, terminal classifications, payloads, and normalization
and redaction policy meaning. Unknown or skipped versions fail closed.
