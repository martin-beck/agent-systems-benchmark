# Offline first-customer release workflow

The release input is one clean exact Git revision. Build the pinned release
binary, then run `tools/release/build_manifest.py` with that revision and the
binary path. The command creates a private output directory containing the
unsigned-release manifest, executable, and deterministic `SHA256SUMS` file.
It rejects dirty trees, revision mismatches, symlinked payloads, and existing
outputs. It performs no network access and records no host paths, credentials,
prompts, or logs.

Supply-chain checks use `tools/quality/run_pinned_cargo_gates.py` with an
operator-provisioned offline directory and `config/release-tools.json`. Both
tool versions and exact binary SHA-256 values are required; placeholders are
not a valid release input. Missing, replaced, or version-drifting tools fail
closed before Cargo runs.

`unsigned-release` is an explicit development/publication profile. It is not
accepted by the default signed runtime-bundle verifier and does not claim a
cryptographic release signature. A future signed publication may add a
detached SSHSIG over the exact manifest bytes without changing the manifest or
payload digest. Tags must be created only after exact-head CI is green and
must never be force-updated.
