# TLA+ tool source-build provenance

ASB qualifies the TLA+ v1.8.0 tool from exact source commit
`b123b22654942bd7f8b1bcadcc47da4ee2cf4c0e`, not from the mutable v1.8.0
prerelease asset. The release repeatedly replaced or deleted asset identities while retaining the
same public URL, so its bytes cannot be an immutable acquisition root.

The reviewed fallback uses the commit-addressed source archive, Apache Ant 1.10.15, and Eclipse
Temurin 17.0.20+8 in the exact Linux amd64 container manifest. It validates the original 31-JAR
inventory, prunes 18 `excluded_source_artifact` entries, and validates the exact 13 `build_input`
allowlist before any Ant invocation. The container has no network, a read-only root filesystem, a
private temporary directory, a non-root identity, and a fixed environment.

The manifest maps every retained JAR digest to an exact coordinate, SPDX expression, and
SHA-256-bound actual terms; coordinates or JAR manifests alone are insufficient. The 18 excluded
artifacts deliberately have no license-applicability claim. `jpf-shell.jar` applicability remains
unknown; it is not asserted to be Apache-2.0 or NOSA-1.3. Ant, Temurin, and container-tool legal
receipts are also exact-bound.

Deleting each retained input must prevent reproduction of the reviewed output. Two independent
clean builds repack payloads with the pinned JDK `jar`, no generated manifest, and the ZIP minimum
timestamp, producing identical 4,512,486-byte artifacts with SHA-256
`8c200a88d151c6c183c8dbc57a6b633d135e7a2b18242a3afbf243a9e4b68d3e`.

This evidence is restricted to the Linux amd64 pruned 13-input closure. It does not qualify the 18
excluded binaries, prove their non-use by other targets, establish upstream endorsement or semantic
correctness, demonstrate native portability, or constitute an unbounded formal result. AR-0877
separately owns cache acquisition and workflow integration after immutable review.
