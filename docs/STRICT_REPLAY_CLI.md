# Strict offline replay CLI contract

`StrictReplayPlanV1` is the CLI consumer boundary for replay. It contains a content digest and
relative cassette path, but never an absolute host path, provider URL, credential locator, or shell
command. The caller supplies an explicit artifact root; resolution rejects traversal, symlinks,
non-regular files, size or digest mismatches, malformed cassettes, and unknown JSON fields.

After the bytes are verified, `asb-cli::replay_contract::resolve_strict_replay` constructs the
authenticated `StrictReplayLaunchRecord`. The runtime-owned sidecar/sandbox seam consumes this
record to issue a `SidecarHandoff`; the CLI cannot forge namespace readiness or provider egress
capability. The record requires `egress: loopback_only`, a bounded timeout, and pinned route,
workload, attempt, and command identities. There is no live-provider fallback.

The checked-in positive fixture is
`crates/asb-cli/fixtures/v1/strict-replay-plan.json`. Focused unit tests cover successful
resolution and traversal, digest, schema-version, and malformed-input rejection entirely offline.
