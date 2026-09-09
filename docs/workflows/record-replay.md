# Record once, replay offline

Recording is an explicit choice because it can expose provider traffic and may
incur cost. Replay is strict and offline; it is useful for deterministic
workflow checks, not for claiming fresh model quality.

## CLI route

Prepare a bounded `RecordingCapture` JSON envelope with explicit `record`,
`network`, and (when applicable) `cost` acknowledgements. Then run:

```sh
asb record /absolute/path/CAPTURE.json /absolute/path/CASSETTE.json
asb replay /absolute/path/CASSETTE.json PROVIDER_PROFILE_SHA256 codex
```

`record` atomically seals the cassette and prints metadata only. The cassette
is immutable content-addressed evidence; keep its path and SHA-256 in the run
record. `replay` requires an exact provider-profile and agent match and denies
provider network access before execution.

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
with their typed reason.

## Evidence boundary

Replay proves an exact cassette/provider/agent path with network denied. It does
not prove current provider availability, current model quality, or native
platform performance. Preserve both live and replay manifests when making a
comparison claim.
