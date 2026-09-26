# Record once, replay offline

Recording is an explicit choice because it can expose provider traffic and may
incur cost. Replay is strict and offline; it is useful for deterministic
workflow checks, not for claiming fresh model quality. The CI example below is
entirely synthetic: it reads a checked-in cassette and never records traffic,
starts a provider, or opens a network connection.

## Offline tutorial contract

The public fixture at
`crates/asb-replay/fixtures/v1/buffered.json` is the only input used by this
tutorial. It has dialect `synthetic`, contains the redaction policy and
selector digest that were applied before persistence, and carries a SHA-256
root in its `integrity` object. Treat those fields as provenance, not as proof
that a provider was contacted. A real recording must be reviewed and redacted
before it is sealed; never put credentials, raw authorization headers, or
unbounded prompts in a cassette.

The syntax-checked sequence is:

```text
fixture = crates/asb-replay/fixtures/v1/buffered.json
assert fixture.contents.interactions[0].dialect == synthetic
assert sha256(canonical(fixture.contents)) == fixture.integrity.digest
assert fixture.contents.redaction.selector_sha256 is present
assert replay_source == fixture.integrity.digest
assert network == denied
assert provider_fallback == forbidden
```

The assertions are deliberately data-only. They validate the cassette schema,
the content/integrity relationship, redaction provenance, deterministic
selection, and the network boundary; they do not invoke `asb record`,
`asb replay`, an agent, or an LLM service. A malformed JSON object, an unknown
field, a changed selector digest, or a mismatched cassette root must fail
closed. Replay failure is returned to the caller; it never falls back to a
provider.

## CLI route

Prepare a bounded `RecordingCapture` JSON envelope with explicit `record`,
`network`, and (when applicable) `cost` acknowledgements. Then run:

```sh
asb record-live /absolute/path/CAPTURE.json /absolute/path/CASSETTE.json \
  --local-mock --confirm-record
asb replay-offline /absolute/path/CASSETTE.json PROVIDER_PROFILE_SHA256 codex
```

`record-live` requires the explicit `--confirm-record` opt-in and atomically
seals the cassette. With `--local-mock`, qualification is credential-free and
does not contact a provider. The cassette is immutable content-addressed
evidence; keep its path and SHA-256 in the run record. Redaction happens before
sealing and the selector digest is retained as provenance. `replay-offline`
requires runtime-issued authority plus an exact provider-profile, agent, and
cassette root match and denies provider network access before execution. If no
exact cassette is available, it returns an error—there is no live-provider
fallback, even when replay is unavailable.
Production provider capture remains a separately supervised runtime integration;
this local/mock route intentionally does not claim external reachability.

To compare live and replay results, first produce two terminal run directories,
then use:

```sh
asb report /absolute/path/live-run
asb report /absolute/path/replay-run
asb compare /absolute/path/live-run /absolute/path/replay-run
```

Treat `comparable: false`, unavailable reasons, redaction, truncation, or a
different profile as a hard boundary—not as a performance difference.

## TUI route

`RecordingWorkflow` exposes the same policy without exposing captured content:

1. `select_live_recording` chooses a live source only after provider preflight.
2. `select_replay(CASSETTE_SHA256)` accepts only an exact compatible cassette.
3. For live recording, explicitly acknowledge network, cost, and persistence
   with `acknowledge_network`, `acknowledge_cost`, and
   `acknowledge_recording`.
4. `review` shows `live_recording` or `strict_replay`, the network policy, and
   unavailable near matches before confirmation.

The negotiated renderer must not show prompts, responses, or credential values
as ordinary UI fields. Nearby but incompatible cassettes remain unavailable
with their typed reason. The TUI route is descriptive only for this tutorial;
the offline CI check does not launch `asb-tui` or a backend.

## Evidence boundary

Replay proves an exact cassette/provider/agent path with network denied. It does
not prove current provider availability, current model quality, or native
platform performance. Preserve both live and replay manifests when making a
comparison claim.
