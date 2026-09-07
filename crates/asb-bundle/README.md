# ASB runtime bundle verification

asb-bundle defines the signed version-1 manifest used by reproducible agent runtime bundles and
verifies it without network access. The signature is an OpenSSH SSHSIG over the exact
manifest.json bytes in namespace asb-runtime-bundle-v1; trust is supplied explicitly as an
allowed-signers file and principal. Verification never downloads keys, packages, SBOMs, or
license data. The caller must also supply the expected SHA-256 of ssh-keygen, preventing an
untrusted replacement executable from certifying arbitrary content.

The signed manifest binds the target OS, architecture, libc family/version, entrypoint, every
regular payload file, executable bits, per-file license expression/evidence, SPDX 2.3 JSON, and
CycloneDX 1.6 JSON. Both SBOMs must describe exactly the same payload paths, hashes, and license
expressions. Missing, extra, duplicate, reordered, symlinked, non-regular, oversized, or modified
content fails closed.

The canonical content digest hashes the strictly path-sorted artifacts. For each artifact it
hashes a big-endian 64-bit byte length followed by the bytes of, in order: path, decimal size,
lowercase SHA-256, executable marker (0 or 1), and SPDX expression. It then hashes the big-endian
license-evidence count and the same length-plus-bytes encoding for every evidence path. The
verifier accepts at most 100,000 artifacts, 64 GiB of declared payload, a 1 MiB manifest and
signature, and 16 MiB per SBOM.

The verifier proves integrity relative to the supplied trusted key and exact target identity. It
does not prove that upstream source is benign, that a license expression is legally correct, that
a libc version is ABI-compatible beyond exact equality, or that a concurrently attacker-mutated
directory is safe to execute. Installation must verify a quiescent private staging directory and
atomically publish the verified tree; sandboxing and process containment remain runtime concerns.
