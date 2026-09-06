# Replay dependency evaluation

Status: AR-0501 evidence snapshot, 2026-09-06.

This evaluation selects an implementation direction; it does not claim that
ASB replay is implemented or that any provider dialect is supported. The
executable synthetic fixture in tools/replay-spike defines the common cases.
Raw model traffic and credentials were never used.

## Decision

Implement a small, safe Rust provider-boundary recorder and replay service in
AR-0502 through AR-0504. Do not add an evaluated candidate as a runtime
dependency. Buffr is the closest behavioral reference and remains useful as an
independent test oracle, but adapting it would retain global replay cursors,
transport-chunk timing, unbounded reads, and incomplete redaction. The other
candidates are generic HTTP tools or language/process-specific test helpers
and require a second ASB state machine to supply the missing guarantees.

The Rust implementation must:

- parse each provider dialect and reject a stream without its required terminal
  event;
- separate immutable cassette data from per-attempt, per-session cursors;
- match every response-affecting field, including tools and causal references,
  under a versioned normalization policy;
- store SSE event boundaries and monotonic offsets separately from transport
  chunks;
- bound request, response, event, decompression, and cassette sizes;
- redact headers, URLs, and configured structured body fields before durable
  writes, preserving distinct stable placeholders;
- provide strict replay with no outbound fallback and report unused, exhausted,
  corrupt, truncated, cancelled, and unmatched trajectories;
- expose immediate, fixed, original, and seeded pacing as separate measured
  modes and measure replay-service headroom.

## Method and evidence classes

The common catalog covers buffered JSON; ordered SSE text and tool-call events;
causal response IDs; parallel identical request shapes with distinct sessions;
original event spacing; a provider error; semantic truncation/cancellation;
malformed and unmatched requests; offline replay; credential markers; and
fail-closed behavior.

- **Boundary** means the candidate recorded from the synthetic origin, the
  origin was stopped, and the candidate replayed the request.
- **Upstream** means focused tests in the pinned source tree executed locally.
- **Source** means implementation code, not a README claim, establishes the
  result.
- **Missing** means the behavior is absent or the evidence cannot establish it.

Buffr, llmtape, and promptecho reached the complete synthetic
record/offline-replay boundary. Mitmproxy and WireMock reached focused upstream
record/replay tests plus code inspection. Agrepl did not reach execution
because its pinned tests do not compile. A source-only cell is not a
compatibility claim.

## Pinned provenance and licensing

| Candidate | Inspected revision | License evidence | Executed baseline |
| --- | --- | --- | --- |
| [buffr](https://github.com/RobinBially/buffr) | ccae7057c99e08049802238a14deea2906013063 | MIT; LICENSE SHA-256 e100ff14f0690964ab682a68d9c67325726743fe0c4bf1c4caa81ca24395cd6e | All six Go packages passed |
| [mitmproxy](https://github.com/mitmproxy/mitmproxy) | 2ac5b089d953585c66026a53f678270e094e48e5 | MIT text; LICENSE SHA-256 cc4ab04ed7180af6c0ede7147f9ec2aa05212c088d7b3c8e38bbffe91c9c8f82 | 23 server-playback tests passed |
| [WireMock](https://github.com/wiremock/wiremock) | d7e390f3fdc56b6356d4e5106d0896a5a9ec305c | Apache-2.0 plus NOTICE; LICENSE.txt SHA-256 cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30 | Focused record API and response-delay tests passed |
| [llmtape](https://github.com/Ayubjon/llmtape) | 78be276b95b8308996f350115fa0bc615caa3324 | MIT; LICENSE SHA-256 b3a9ecf57be98b88444ae976e9ccbff63fcfa517ba405e7f73517fd42820d562 | 29 Node tests passed |
| [promptecho](https://github.com/shwetank/promptecho) | 3b6ed64394180cf17137f661dafd20babb356d0a | MIT; LICENSE SHA-256 855702270a8d8e20a9f9ad7e6eb35804812bcd3d0157d6e9018e44c503415bcb | 66 Python tests passed |
| [agrepl](https://github.com/taiwrash/agrepl) | e61b866984cdc45db1be91c0509c76b1cb417d28 | **No project-level license file found.** Tracked website dependency licenses do not license agrepl. | Focused Go tests failed to compile |

License compatibility here is not dependency approval. Agrepl cannot be copied
or distributed based on the paper's MIT statement when the pinned source tree
does not contain that license grant.

## Conformance findings

| Candidate | Buffered | SSE, tools, causal IDs | Parallel sessions | Pacing and cancellation | Offline miss | Redaction and bounds |
| --- | --- | --- | --- | --- | --- | --- |
| buffr | Boundary pass | Boundary byte pass; timing follows transport chunks, not SSE events | Missing: one shared cyclic cursor per key | Replays millisecond chunk delays; accepted semantic truncation | Boundary pass, HTTP 599 | Header redaction passed; body marker persisted; reads unbounded |
| mitmproxy | Upstream pass | Source preserves buffered body; no event-offset cassette model | Missing: global flow list popped in arrival order | Missing event pacing and provider-terminal validation | Unsafe default forwards unmatched requests | Generic flows retain sensitive content without custom filters |
| WireMock | Upstream pass | Recorded response becomes a buffered stub body | Missing: server-global scenarios/mappings | Configured chunked-dribble delay is synthetic | Failure is configurable separately from proxying | Response log size can be limited; no provider-aware default redaction |
| llmtape | Boundary pass | Missing: replay JSON-encodes buffered SSE text | Missing: first fingerprint match | Missing | Boundary pass, HTTP 404 | Body marker persisted; whole files/bodies unbounded |
| promptecho | Boundary pass | Boundary byte pass after full buffering; tool matching passes; causal fields omitted | Missing: process-global patch and first match | Record waits for completion; replay immediate | Boundary pass in mode none | URL values redact; body marker persists; no ASB-wide bounds |
| agrepl | Not executable | Source buffers complete bodies | Missing: global used-step map | Missing | Source fails by default, optional fallback permits network | Persists headers/body, disables upstream TLS verification, license absent |

### Buffr

The boundary replay reproduced buffered JSON and SSE bytes after the origin was
stopped. Tool-call ID call-alpha and causal ID response-parent survived, and an
unmatched request returned 599. Its cassette stored four interactions,
including the deliberately incomplete stream. It redacted the synthetic
Authorization header but retained the synthetic body marker.

The code has one mutex-protected cursor per request signature and advances it
modulo the entry count. That prevents a data race but is not session-scoped:
simultaneous identical sessions consume one shared sequence. Streaming capture
records each Read result and elapsed whole milliseconds, so transport
coalescing defines cassette boundaries. Request bodies and cassette files are
read without a configured maximum.

### Mitmproxy and WireMock

Mitmproxy server playback tests passed, but flowmap is global and matching pops
the first flow for a hash. Its default server_replay_extra is forward and
response refresh is enabled by default, rewriting date, expiry, last-modified,
and cookie expiry values. Strict kill or fixed-error modes can be configured,
but do not supply session isolation, SSE pacing, or redaction.

The corrected focused WireMock invocation passed recording API and response
delay tests. Recording converts a logged response into a static response
definition backed by buffered bytes. Chunked-dribble delay divides a body into
configured chunks over a configured duration; it does not recover origin SSE
event boundaries or offsets. Extensions could add transforms, but ASB would
still implement matching, cursors, completion checks, redaction, and pacing.

### Llmtape and promptecho

Llmtape concatenates response chunks, then JSON-encodes a non-JSON response on
replay, so the SSE bytes changed. Its fingerprint includes only model,
messages, and system while intentionally ignoring response-affecting values
such as temperature, stream settings, seed, and penalties. Tool definitions
and causal IDs changed without a miss. Buffered replay and an unmatched 404
worked.

Promptecho preserved completed SSE bytes and detected a changed tool definition.
It nevertheless reads the full real response before returning a new buffered
response, stores no event timing, and rejoins events immediately on replay. In
one bounded run, record took about 138 ms and replay about 24 ms; these single
observations demonstrate mechanism, not a performance distribution.

Session and previous_response_id are outside its default fingerprint. A request
changed from session alpha and parent response-parent to session beta and
parent different-parent but replayed the original trajectory. It redacted
query values, retained the request-body marker, and accepted a stream with no
provider completion event.

### Agrepl

At the pin, interceptor tests fail to compile because fixtures initialize
byte-vector request and response fields with strings. Source inspection cannot
be upgraded to compatibility evidence. The interceptor buffers bodies, tracks
one global used-step map, persists full headers and bodies, allows optional
network fallback, and creates an upstream transport with TLS verification
disabled. These are blockers even if the build is repaired.

## Literature boundary

[AgentRR](https://arxiv.org/abs/2505.17716) records and abstracts agent
experience so later agents can reuse workflows and constraints. That differs
from replaying exact provider responses while the real agent and tools execute.
Its check function motivates independent validation, but its results do not
establish HTTP/SSE fidelity or Linux timing repeatability.

[Deterministic Replay for AI Agent Systems](https://arxiv.org/abs/2607.16200)
states transport-level isolation and fidelity results for agrepl. The pinned
implementation does not justify adopting those claims here: tests fail to
compile, the checkout lacks the stated MIT license, fallback can be enabled,
TLS verification is disabled upstream, and buffered bodies lose stream timing.

[Application-Integrated Record-Replay of Distributed
Systems](https://www2.eecs.berkeley.edu/Pubs/TechRpts/2024/EECS-2024-4.html)
motivates recording at a boundary exposing the causality required for replay
without serializing unrelated work. For ASB, that means provider events and
per-session causal references, not tool execution or the measured operating
system.

## Evidence limits

- Executable evidence is x86_64 Linux only; it proves no aarch64 or
  distribution support.
- No real agent or provider was contacted. AR-0505 must validate each exact
  client and dialect through live record then network-denied replay.
- Loopback synthetic traffic does not validate TLS interception, certificates,
  WebSockets, HTTP/2, proxies, authentication, or provider compatibility.
- Pacing observations are functional spikes, not load/headroom measurements.
- No candidate was fuzzed or mutation-tested. Later replay assurance must cover
  parser limits, corrupt cassettes, decompression bombs, redaction bypasses,
  cancellation races, and cursor interleavings.
- Earlier handoffctl bash-stdin commands share an argv-only digest and are not
  unique provenance identifiers. Their named outcomes above and retained
  synthetic artifacts were independently inspected; later commands avoid that
  form.
- External checkouts, caches, binaries, cassettes, and synthetic raw logs stayed
  outside Git. Only the reviewed fixture and this concise result enter history.
