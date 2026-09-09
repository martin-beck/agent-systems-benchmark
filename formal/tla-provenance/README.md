# Deterministic TLA+ source build

This directory binds the exact reviewed TLA+ source commit, the pinned offline build container,
Apache Ant, the original 31-JAR source inventory, and the deterministic output. The inventory is
closed as 13 `build_input` JARs and 18 `excluded_source_artifact` JARs. `build.sh` validates all
31, deletes every excluded artifact, and validates the exact 13-JAR allowlist before its first Ant
invocation. It proves each retained JAR is necessary and builds twice offline with identical bytes.

License evidence is not inferred from coordinates. Each retained input maps to an exact coordinate
and digest-bound actual terms. Excluded artifacts carry only path, digest, and classification; in
particular, `jpf/jpf-shell.jar` license/source applicability remains unknown and no Apache or NOSA
claim is made for it. The build also
verifies Apache Ant's LICENSE and NOTICE, Temurin legal files, and container tool receipts.

JavaCC 4.0 and prettier4j 0.3.2 lack an embedded usable license receipt, so their manifest entries
also bind the authoritative upstream repository, immutable tag and commit, commit-addressed archive
size and digest, license path and digest, receipt transformation, and package-applicability file
digest. JavaCC's local receipt strips only per-line trailing ASCII whitespace from the upstream
`LICENSE`;
prettier4j's receipt is byte-identical to upstream. These closed fields are checked semantically and
covered by provenance mutation negatives.

The qualification is intentionally limited to Linux amd64 and Eclipse Temurin 17.0.20+8. It proves
that the pruned 13-input closure produced the recorded bytes twice. It does not qualify the 18
excluded binaries, make the mutable upstream v1.8.0 prerelease asset immutable, prove excluded
non-use outside this exact recipe, or imply upstream endorsement or TLA+ correctness.
