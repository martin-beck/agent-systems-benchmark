# TLA+ tool source-build provenance

ASB qualifies the TLA+ v1.8.0 tool from exact source commit
`b123b22654942bd7f8b1bcadcc47da4ee2cf4c0e`, not from the mutable v1.8.0
prerelease asset. The release repeatedly replaced or deleted asset identities while retaining the
same public URL, so its bytes cannot be an immutable acquisition root.

The reviewed fallback uses the commit-addressed source archive, Apache Ant 1.10.15, all 31 vendored
JARs, and Eclipse Temurin 17.0.20+8 in the exact Linux amd64 container manifest recorded in
`formal/tla-provenance/source-build.toml`. Every build input is verified before use and the build
container has no network, a read-only root filesystem, a private temporary directory, a non-root
identity, and a fixed environment. No Maven or Gradle repository participates.

The manifest maps every vendored JAR digest to an SPDX expression and a SHA-256-bound actual license
text in the pinned closure; package coordinates or JAR manifests alone are insufficient. The build
also checks Ant's LICENSE/NOTICE and the exact Temurin and container-tool legal receipts before
compilation. Missing, substituted, metadata-only, or non-license receipts fail closed.

Two independent clean container builds produced different raw ZIP metadata but identical 2,087-file
payload trees. Repacking those payloads with the pinned JDK `jar`, no generated manifest, and the ZIP
minimum timestamp produced identical 4,512,486-byte artifacts with SHA-256
`8c200a88d151c6c183c8dbc57a6b633d135e7a2b18242a3afbf243a9e4b68d3e`.

This evidence is restricted to the declared Linux amd64 build closure. It proves deterministic bytes
and license-complete input accounting, not upstream endorsement, semantic correctness, native
portability, or an unbounded formal result. AR-0877 separately owns cache acquisition and workflow
integration after this manifest receives immutable review.
