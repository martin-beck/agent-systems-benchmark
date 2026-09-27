# Self-contained package qualification (non-production)

This workflow exists for credential-free development when the externally signed
ASB runtime package is unavailable. It uses the existing `asb-bundle` offline
verifier test-key fixture and the public CLI journey in an owner-only temporary
XDG root. It is **non-production qualification** and is **never release
evidence**.

Run the deterministic fixture gates from the repository root:

```sh
cargo test --locked -p asb-bundle --test offline_verifier
cargo test --locked -p asb-cli --test package_qualification_fixture
cargo test --locked -p asb-cli --test workflow_transcript
```

The verifier fixture proves signed manifest, checksum, SBOM, provenance, and
runtime-component handling, including fail-closed tamper and policy cases. The
CLI fixture then runs `doctor` and `setup --format=json` with isolated owner-only
XDG roots; the existing workflow transcript supplies local/mock run, sweep,
record, strict replay, comparison, and cleanup evidence.

No provider, credential, network, asb-tui, or production release signer is used.
The test key and generated package are deliberately not customer artifacts.
Actual customer release still requires the exact externally signed package,
allowed-signers policy, provenance, checksums, and release gates documented in
[`RUNTIME_BUNDLES.md`](../RUNTIME_BUNDLES.md).
