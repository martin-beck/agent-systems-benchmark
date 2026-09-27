# Owner-backed first-customer journey

This is the shortest ASB-only, credential-free qualification route after an
authenticated install. It composes the install/bootstrap contract, setup
preflight, runtime-owned local/mock run and sweep, strict offline replay, and
bounded evidence inspection. It does not contact a provider, require a
credential, launch asb-tui, or claim native ARM support.

## 1. Install and bootstrap

Use the pinned release manifest, detached signature, and allowed-signers file
described in [CLI first run](cli-first-run.md). Keep installation disposable
and owner-only. Before any workload operation, run:

    asb doctor
    asb setup --format=json

Doctor and setup are side-effect bounded. Setup writes only a validated
configuration when explicit provider/model inputs are supplied; it never
stores credential values. For local qualification, use the checked-in
local/mock fixtures and do not provide a live credential reference.

## 2. Run and sweep through the owner

The runtime/control owner is provisioned and enrolled before the ordinary CLI
request. The owner-backed entry accepts only run PLAN or sweep PLAN; it
supplies the local/mock attempt backend and tears down on return. Positive
coverage for both operations, malformed-entry rejection, bounded/private
output, and reuse-after-teardown is exercised by:

    cargo test --locked -p asb-cli runtime_local_mock_owner -- --nocapture

The result and progress channels are bounded and must not contain plan paths,
credential digests, prompts, transcripts, or raw captures.

## 3. Replay, comparison, and recovery

Use the checked synthetic cassette and the runtime-issued replay authority:

    cargo test --locked -p asb-cli replay -- --nocapture
    cargo test --locked -p asb-runtime launch_factory -- --nocapture
    cargo test --locked -p asb-runtime live_service -- --nocapture

Replay is strict and offline. The cassette, provider profile, agent, receipt,
and authority identity must match. Missing, stale, copied, tampered, replayed,
or teardown-revoked authority fails closed before execution effects. Inspect
terminal report evidence and compare only runs marked comparable. Cancellation,
restart reconciliation, idempotent cleanup, and owner teardown are runtime
tests, not live-provider evidence.

## Qualification boundary

This journey qualifies ASB's owner-backed local/mock/replay path and its
machine-readable evidence boundary. It does not qualify provider reachability,
fresh model quality, paid API behavior, native platform performance, or the
paired asb-tui journey. Any failed required gate remains a failure; local
fixtures cannot be promoted to a live claim.
