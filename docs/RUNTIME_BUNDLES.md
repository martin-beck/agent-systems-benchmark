# Runtime bundle manifests and offline verification

ASB agent installations use the version-1 contract in
[asb-bundle](../crates/asb-bundle/README.md). A bundle directory contains exactly:

- manifest.json and, for the `signed` profile, its detached manifest.json.sig SSHSIG;
- the SPDX 2.3 and CycloneDX 1.6 documents named by the manifest; and
- every regular payload and license-evidence file in the sorted artifact inventory.

The release process creates all payloads in a private quiescent staging directory, generates both
SBOMs from the same complete inventory, calculates the documented content digest, serializes the
manifest, and signs its exact bytes:

```sh
ssh-keygen -Y sign -f RELEASE_KEY -n asb-runtime-bundle-v1 manifest.json
```

The manifest's explicit `profile` and `signature_status` fields distinguish the normal `signed`
profile from opt-in `unsigned-development` and `unsigned-release` profiles. Unsigned profiles
retain complete content hashes, SBOMs, license evidence, target identity, and provenance; they
omit only `manifest.json.sig`. The default verifier and all formal qualification paths reject
unsigned profiles. A caller may deliberately consume one for development or a tagged release
with `asb-bundle-verify ... --profile unsigned-development` or `--profile unsigned-release`.

Release keys are never part of a bundle. Installers carry an independently provisioned OpenSSH
allowed-signers file, required principal, exact trusted ssh-keygen path and its SHA-256. They
invoke the verifier with an exact platform identity:

```sh
asb-bundle-verify BUNDLE ALLOWED_SIGNERS PRINCIPAL SSH_KEYGEN SSH_KEYGEN_SHA256 \
  linux x86_64 glibc 2.39
```

The verifier performs no network operation. It verifies the signature before accepting manifest
semantics using a two-second process-group-bounded ssh-keygen invocation with no inherited
environment or retained subprocess output. It requires exact target equality, rejects unknown manifest fields, and then checks the
complete directory inventory, file sizes/hashes/exact safe permission modes, canonical content digest, both
SBOM hashes, and exact per-file path/hash/license parity. Its output contains only bounded public
identities and digests; failures never include file contents, signer data, or subprocess output.

Verification is an integrity and identity decision, not an execution sandbox or a legal license
opinion. The current implementation supports Unix permission bits and uses Linux O_NOFOLLOW when
opening files. It rejects stable symlinks and special files, but it does not make a hostile
same-UID directory concurrently mutated during verification safe to execute. The installer must
own and keep staging private, verify it while quiescent, and atomically publish it. Platform
compatibility is exact string equality; broader libc ABI compatibility requires separate native
qualification. Bundle creation and per-agent transitive package acquisition remain AR-0316 work.
The configured ssh-keygen and allowed-signers paths are also trusted operator-owned inputs and
must remain quiescent for the duration of verification.

## Reproducible assembly

The release operator builds the pinned supervisor and sidecar in the reviewed release workflow,
then supplies those regular files explicitly to the offline builder. The builder never discovers
or downloads executables and refuses symlinks, missing signing inputs, and an existing output
archive:

```sh
python3 tools/bundle/build_runtime_bundle.py \
  --bundle-version 1.0.0 --os linux --arch x86_64 --libc glibc --libc-version 2.39 \
  --supervisor target/release/asb_loopback_supervisor \
  --sidecar target/release/asb_loopback_sidecar \
  --key RELEASE_KEY --allowed-signers ALLOWED_SIGNERS \
  --principal asb-release --ssh-keygen-sha256 SSH_KEYGEN_SHA256 \
  --output dist/asb-runtime-1.0.0-linux-x86_64.tar.gz
```

Payload inventory and both SBOMs are generated before the canonical manifest is signed. Archive
metadata uses epoch timestamps, zero ownership, sorted entries, and atomic replacement, so the
same staged inputs produce byte-identical output. `asb-bundle-verify` validates the staged tree
before archiving; private keys are read only by the operator's `ssh-keygen` process and are never
copied into the bundle or retained as evidence.
