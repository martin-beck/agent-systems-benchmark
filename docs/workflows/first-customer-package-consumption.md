# First-customer package consumption

This qualification consumes one exact ASB release bundle in a disposable
owner-only directory. It verifies the package boundary before exercising the
owner-backed local/mock/replay journey. It is credential-free and offline
after the bundle is present; it does not contact a provider, launch asb-tui,
or claim native ARM support.

## Verify before install

Build or obtain the exact release bundle and retain its commit, manifest,
checksums, SBOM, provenance, and signature-policy result. Verify the bundle
before copying it into an installation root:

    asb-bundle-verify BUNDLE ALLOWED_SIGNERS PRINCIPAL SSH_KEYGEN SSH_KEYGEN_SHA256

The verifier must reject altered payloads, unknown manifest fields, target
drift, missing integrity documents, and an unsigned profile under the default
policy. An unsigned development profile is never release evidence.

## Disposable install and operation

Install only into a temporary owner-only XDG root, then run:

    asb doctor
    asb setup --format=json

Use the owner-backed local/mock path for both ordinary operations and inspect
only bounded JSON result/receipt fields:

    cargo test --locked -p asb-cli runtime_local_mock_owner -- --nocapture

The owner is enrolled before dispatch and torn down after each bounded
operation. Reuse after teardown, malformed entry shapes, unavailable owner
sources, and private output are hostile cases; no credential, prompt,
transcript, raw capture, or host path may appear in evidence.

## Replay and cleanup

Consume the checked synthetic cassette with runtime-issued authority and
compare only terminal results marked comparable:

    cargo test --locked -p asb-cli replay -- --nocapture
    cargo test --locked -p asb-runtime launch_factory -- --nocapture
    cargo test --locked -p asb-runtime live_service -- --nocapture

Strict replay denies provider egress and rejects missing, stale, copied,
tampered, replayed, or teardown-revoked authority before effects. After the
walkthrough, remove only the disposable installation root and verify that no
owner/backend resource remains. Cleanup does not purge retained release,
trust, or evidence state implicitly.

## Boundary

This proves fresh consumption of the ASB package and owner-backed
local/mock/replay path. It does not prove provider reachability, fresh model
quality, paid API behavior, paired asb-tui behavior, or native platform
performance.
