# Deterministic TLA+ source build

This directory binds the exact reviewed TLA+ source commit, the pinned offline build container,
Apache Ant, all 31 vendored JAR inputs and their license evidence, and the deterministic output.
`build.sh` consumes an already-populated cache and runs the build with container networking denied;
it never downloads inputs. `verify.sh` can validate the closed manifest alone or the complete
source-tree/output receipt.

License evidence is not inferred from coordinates. Each dependency maps to a digest-bound actual
license text already present in the pinned source closure. The build also verifies Apache Ant's
LICENSE and NOTICE plus the Temurin JDK legal files and the container-resident copyright receipts
for the exact shell, archive, file-discovery, and core utilities used by the recipe.

The qualification is intentionally limited to Linux amd64 and Eclipse Temurin 17.0.20+8. It proves
that the declared closure produced the recorded bytes twice. It does not make the mutable upstream
v1.8.0 prerelease asset immutable and does not imply upstream endorsement or TLA+ correctness.
