# Portable OCI build-image identity

The deterministic TLA+ source build invokes exactly the reviewed
`eclipse-temurin@sha256:c0d1549d1e0f5fa5b83622ec0033b00456107e0b1d0cfcce4c1d831532ce621e`
image on `linux/amd64`. Docker's local configuration ID is not the registry manifest digest and may
differ across conforming engines. It is therefore validated only as a well-formed informational
identity.

Before any container runs, the builder pipes a minimal JSON projection from `docker image inspect`
directly into a verifier that reads at most 16,385 bytes. Inspect diagnostics are suppressed at this
privacy boundary and failures are replaced with bounded generic text. The verifier requires the
exact name-at-digest reference as the sole repository digest, plus exact `linux` and `amd64` fields.
Unknown, duplicate, missing, malformed, oversized, trailing, tag-only, local-ID-only, wrong-digest,
wrong-name, or wrong-platform evidence fails before any container effect.

The existing `--pull never`, `--network none`, read-only, non-root build and exact output-verification
boundaries remain unchanged. This verifies one repository identity and platform representation; it
does not establish registry or daemon trust, qualify tags or other architectures, or prove TLA+
correctness.
