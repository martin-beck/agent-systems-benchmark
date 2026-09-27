# Owner-backed first-customer qualification

This records the credential-free, provider-free qualification slice for the
ordinary ASB CLI. It proves the runtime-owned owner boundary, bounded
evidence, strict replay, recovery semantics, and teardown using local
fixtures only. Provider reachability, credentials, native ARM, and asb-tui
are not prerequisites or claims of this slice.

## Qualified path

The runtime/control owner is provisioned and enrolled before the CLI receives
the request. The CLI accepts only the ordinary run PLAN or sweep PLAN shape,
while the owner supplies the local/mock attempt backend. The helper tears the
owner down on return; reuse after teardown and malformed entry shapes fail
before a result is created.

The focused qualification command is:

    cargo test --locked -p asb-cli runtime_local_mock_owner -- --nocapture

Positive tests cover both run and sweep. Negative tests cover owner reuse
after teardown, an unavailable runtime source, an unqualified entry shape, and
bounded/private output. Result output is a JSON envelope; the test bounds it
and rejects plan paths, credential digests, prompts, transcripts, and raw
captures.

## Replay and recovery evidence

Strict replay is exercised by the existing run_with_replay_authority and
launch-factory tests. Replay authority is runtime-issued and one-shot; tests
reject missing or copied attestations, stale generations, tampered receipts,
replayed receipts, and teardown reuse. Replay has no provider fallback or
network requirement. Runtime recovery tests cover cancellation fencing,
interrupted-attempt reconciliation, idempotent cleanup, and revocation at
teardown. These tests qualify the controlled runtime path, not fresh provider
quality.

Applicable focused commands are:

    cargo test --locked -p asb-runtime control_owner launch_factory live_service
    cargo test --locked -p asb-cli runtime_local_mock_owner replay

## Evidence boundary

Evidence may contain stable identifiers, bounded result fields, hashes, exit
codes, and explicit unavailable reasons. It must not contain credential
values, prompts, responses, transcripts, private host paths, raw captures, or
live-provider data. A failed gate remains a failure; no fixture, mock, or
replay result is promoted to a live-provider or native-platform claim.
Cleanup is verified by owner teardown and by the absence of a result for
rejected entry shapes.
