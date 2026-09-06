# ASB extension protocol v1

External extensions are executables speaking JSON-RPC 2.0 as newline-delimited
UTF-8 JSON over stdin/stdout. Logs belong on stderr. Rust traits may be used for
built-ins, but Rust dynamic libraries are not a stable or supported plugin ABI.

The first request must be `negotiate`. Major version 1 is required; peers select
the lower supported minor and the stricter nonzero frame, deadline and in-flight
limits. Callers enforce the negotiated timeout against a monotonic clock, reserve
request capacity before external effects, bound queued I/O, and terminate a stalled
extension. The framing helpers enforce the hard 16 MiB ceiling even if a caller
supplies a larger value and stop serialization before an oversized buffer is built;
the admission helper enforces request-count bounds.
Process deadlines and process-tree cancellation remain runtime-layer obligations.

Application errors are `-32001` incompatible version, `-32002` oversized frame,
`-32003` deadline exceeded, `-32004` unavailable capability, `-32005` stale
attempt, `-32006` duplicate request, and `-32007` resource exhaustion. Standard
JSON-RPC parse/request/method/params errors retain `-32700` and
`-32600` through `-32602`. Missing capabilities and measurements are unavailable,
never silently supported or zero.

Requests carry IDs and receive exactly one response. Event notifications have no
ID or response and carry session ID, attempt ID, and a strictly increasing
per-attempt sequence. Schemas in `schema/v1` are generated from the Rust types.
Regenerate with:

```sh
cargo run --locked -p asb-protocol --example generate-schemas -- crates/asb-protocol/schema/v1
```

Assurance is deliberately bounded. The terminal result test exhaustively checks
all six combinations of the three terminal states and presence/absence of error
evidence, so it is a finite mechanical invariant of that pure validator. Schema,
framing and subprocess conformance tests are implementation tests, not proofs.
Process deadline enforcement, scheduler backpressure, process-tree cancellation,
and native-platform behavior are environmental/runtime obligations outside this
crate and remain unclaimed here.

## Experiment identity v1

The experiment manifest binds comparisons to agent implementation and binary,
model/provider settings, tool policy, workload and scorer, image and dependency
content, native platform topology, and cache/load/retry/replay controls. Provider
credentials, endpoint URLs, prompts, and host paths are never manifest fields.
Provider-specific settings are represented only by the SHA-256 of a separately
canonicalized, credential-free settings object.

The generated schema enforces character-count bounds, feasible numeric ranges, and live/replay
cassette consistency. Runtime validation additionally enforces the stricter 1,024-byte UTF-8 limit
and rejects leading/trailing whitespace and control characters; portable JSON Schema cannot
express those exact Rust UTF-8 semantics.

The experiment address is SHA-256 over the domain bytes
`asb-experiment-v1` followed by a zero byte and every nested identity field in
manifest declaration order. Text is encoded as an unsigned 64-bit big-endian
UTF-8 byte length followed by those bytes. Integers are fixed-width big-endian;
optional values use a one-byte 0/1 tag. Cache-state tags are cold=0, warm=1, and
uncontrolled=2; replay-mode tags are live=0 and replay=1. The declared experiment address is
excluded from its own preimage. The validator rejects malformed component hashes,
unbounded text, zero topology/deadline values, inconsistent live/replay cassette
state, and an address mismatch.
