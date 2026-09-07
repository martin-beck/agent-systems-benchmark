# asb-replay cassette contracts

`asb-replay` defines the immutable, versioned, privacy-reviewed response
cassette boundary and a strict inbound-only replay service. AR-0503 adds
per-session matching and immediate buffered/SSE delivery. AR-0504 adds a
monotonic pacing and calibration boundary. Neither provides transparent
interception or a real-client compatibility claim.

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
values containing control characters also fail closed. Canonical stored header
names contain only lowercase ASCII letters, digits, hyphens, and underscores;
the underscore allowance is required by the pinned OpenDesk client. Other HTTP
token punctuation is deliberately unsupported.
Event boundaries and monotonic offsets are independent of transport chunk sizes.
A streamed response has exactly one terminal classification on its final event.

Default hard ceilings are 256 MiB per cassette, 16 MiB per request, 64 MiB per
buffered response, 16 MiB per event, 65,536 events, 16,384 interactions, and
1,024 headers. Callers may tighten but not raise these limits. Version 1 stores
uncompressed JSON; compressed input is unsupported, avoiding an unbounded
decompression boundary.

Before canonicalizing structured contents, sealing streams them through the
configured cassette-size bound and rejects an aggregate overrun. For an accepted
cassette the implementation still holds the caller-owned structure, a bounded
preflight buffer, a `serde_json::Value` tree, and canonical output during
sealing; the 256 MiB ceiling is an encoded-size limit, not a 256 MiB peak-RSS
guarantee.

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

Strict replay applies authenticated request-body selectors only for comparison:
it substitutes each present incoming selected value with the exact bounded
redaction marker already stored at that pointer. Missing or out-of-bounds
pointers, malformed expected markers, marker-shaped incoming data, and markers
outside selected fields fail closed. Every unselected field, including model,
tools, prior-response identity, and stable option content, remains byte-exact
after canonical JSON encoding. When a selected pointer is inside a duplicated
top-level provider option, pre-persistence redaction synchronizes that option
from the redacted body so the cassette cannot retain the volatile value through
its denormalized request metadata.

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

## Strict replay service

The trusted coordinator selects a cassette session, attempt, and provider
dialect for each isolated listener route. Exact POST endpoints are implemented
for Chat Completions (`/v1/chat/completions`), Responses (`/v1/responses`), and
Messages (`/v1/messages`). These are syntax capabilities only; AR-0505 must
test named real client versions before any compatibility claim.

The Gemini GenerateContent syntax capability is restricted to
`POST /v1beta/models/{model}:streamGenerateContent?alt=sse`, where `{model}` is
1--256 ASCII letters, digits, dots, underscores, or hyphens and must equal the
cassette model. The model appears only in the route. The JSON object has exactly
`contents`, `generationConfig`, `systemInstruction`, and `tools`; the pinned
shape requires captured numeric temperature, topK, and topP values plus an exactly
empty `thinkingConfig` object,
one functionDeclarations container, and description/name/parametersJsonSchema
declarations. Content parts are restricted to text, functionCall plus
thoughtSignature, or functionResponse shapes. The complete canonical request,
including every nested configuration, tool value, semantic text, and tool
identifier, matches exactly. When an SSE response requests a function call, the
next recorded request must contain a functionCall/functionResponse pair whose ID
and name both equal that response. The pinned Gemini 0.58.0 capture has an empty request-body
selector set, which this dialect requires; whole user or system text is not a safe
volatile selector. Marker-shaped incoming values fail closed.

Gemini also admits the exact pinned retry response: HTTP 500 `application/json`,
failed terminal status, no response identity, and one error object containing only
code `500`, a nonempty message, and status `INTERNAL`. The retry consumes its own
ordered interaction, and the immediately following Gemini request must be exactly
identical.

Successful Gemini event responses require HTTP 200, exact `text/event-stream`,
and exactly one terminal `gemini.generate_content.chunk` event. Its closed payload
has one model candidate at index zero with finish reason `STOP`, exactly one
nonempty text or exact functionCall part, and candidate/prompt/total unsigned token
counts whose first two sum to the total. Replay emits the event as one data-only
SSE record, `data: {canonical JSON}\n\n`; it emits neither an `event:` line nor a
`[DONE]` marker. Other Gemini routes, query spellings, body fields,
buffered responses, and model characters are unsupported and fail closed. This
syntax contract alone is not a real Gemini CLI compatibility or network-isolation
claim; those require the separately pinned credential-free journey.

The Chat Completions dialect also admits the two explicitly recorded OpenDesk
catalog probes, `GET /v1/models` and `GET /v1/models/{model}`. The detail suffix
must exactly equal the cassette model, whose identifier is bounded to 256 safe
ASCII bytes. A catalog request has an empty wire body, canonical `null` body,
no options, tools, or causal predecessor, and consumes the same ordered cursor
as completions. Its response must be HTTP 200 buffered `application/json`,
completed without a response ID. A missing Content-Length or the canonical
`Content-Length: 0` is accepted for GET; nonzero or duplicate lengths fail
closed. This is syntax compatibility, not an OpenDesk support claim.

For Chat Completions, an omitted `stream` member can select recorded events only
when the response has exactly one `content-type: text/event-stream` header. An
explicit false value, a non-Boolean value, or any other/missing content type
cannot select events. Existing explicit true and buffered-response rules remain
unchanged.

Matching compares the complete duplicate-free semantic JSON body, origin-form
target, and end-to-end headers. Only `host`, `content-length`, `connection`,
`accept-encoding`, and `user-agent` are explicitly transport-normalized.
Policy-declared sensitive header values match the cassette placeholder while
their names and presence remain exact. Every other difference, malformed body,
unsupported transfer encoding or dialect, unknown route, or exhausted cursor
fails closed without advancing the cursor.
Header count and aggregate head bytes are bounded for both direct and socket
entry points. SSE event-type tokens permit only ASCII letters, digits, dot,
underscore, and hyphen, preventing framing injection from a recomputed cassette.

Each `(session_id, attempt_id)` owns an independent cursor. A socket request
reserves its exact next interaction while writing, so concurrent admission on
that route fails closed while unrelated routes remain available. The cursor is
committed only after every response byte is accepted by the socket writer; a
write error releases the reservation and leaves the interaction retryable.
Incomplete cancellation, synthetic-failure, lateness, and backpressure failures
also release the reservation. If every semantic segment was completely written
before an over-bound duration was observed, the cursor commits even though the
call returns a pacing error, because retrying could duplicate the full response.
Direct `handle` calls consume when they return successfully. Buffered responses
are canonical JSON. Streamed semantic events are emitted immediately as SSE in
cassette order; Chat Completions receives a final `[DONE]` marker. Captured
transport chunks and monotonic offsets are not reproduced. Pacing evidence is
owned by AR-0504.

`StrictReplayService::serve_once` accepts one loopback HTTP/1.1 connection and
never opens an outbound socket. The caller owns listener lifecycle and network
namespace/firewall enforcement. Chunked requests, HTTP/2, WebSockets,
compression, TLS interception, keep-alive pipelining, and non-loopback peers
are unsupported. Inbound-only code does not prove an agent has no alternate
network route.
The returned status lets the coordinator distinguish a served interaction from
a fail-closed local error without inspecting captured payloads. Read and write
timeouts are tightenable in `(0, 30 seconds]`; successful writes mean bytes were
accepted by the local kernel, not that the peer consumed them.

## Pacing and replay headroom

`write_paced_segments` supports four explicit modes. Immediate mode adds no
intentional delay. Fixed mode declares time to first segment and inter-segment
cadence. Original mode uses each cassette event's nondecreasing monotonic offset.
Seeded synthetic mode uses the documented version-one SplitMix64 schedule and an
optional declared failure index; its results are classified `SyntheticScenario`,
while the other modes are `RecordedResponse`. Pacing evidence records desired,
write-start, write-complete, and lateness offsets for every completed segment.
It does not claim wall-clock determinism or transport-packet fidelity.
Every generated or recorded desired offset is capped at five minutes, including
the cumulative fixed and seeded schedules, and segment count is capped by the
cassette event limit before the schedule vector is allocated.

Waiting uses a monotonic clock and observes cooperative cancellation at a bounded
poll interval. Segment write and lateness ceilings fail closed. The loopback TCP
service enforces one absolute deadline over each complete head or semantic-segment
write, including a peer that makes only slow partial progress. The standalone
generic Rust `Write` API cannot interrupt a blocked implementation; its caller
must provide an equivalent transport deadline. Partial bytes and failed or
cancelled runs remain failures and do not prove peer consumption.

`assess_replay_headroom` consumes calibration samples collected independently of
the client sweep. It requires at least one strictly higher replay concurrency,
retains the highest point satisfying completion, queue-delay, pacing-lateness and
CPU/wall limits, and records the first saturated point. A sufficient verdict means
only that the supplied bounded observations show replay-service headroom above the
declared client concurrency. It does not collect CPU/queue data, validate platform
support, or prove that later co-located runs remain uncontaminated.
