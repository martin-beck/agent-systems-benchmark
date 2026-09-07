# Platform manifest and evidence matrix

The versioned sources of truth are
[platforms.json](../platforms/v1/platforms.json) and
[agents.json](../platforms/v1/agents.json). They pin registry indexes, per-architecture
image manifests, libc baselines and agent package metadata as observed on
2026-09-06. Run `python3 tools/platforms/validate_manifests.py` after any update.

## Evidence labels

`planned`, `build-only`, `simulated`, `native-tested` and `unsupported` are
different states. An OCI index proves only that a registry serves a user-space
filesystem for an architecture. It does not identify the host kernel, prove that
the image starts, install an agent, execute a workload or establish native support.
A cross-build is never promoted to `native-tested`. That label requires a digest
for an artifact produced by a run on the named native architecture and kernel.
The artifact must be a bounded, sanitized report produced by
`tools/platforms/native_evidence.py`. The manifest validator checks its SHA-256,
source commit, platform, architecture, kernel, run ID, cgroup v2 and PSI probes,
and passing process, metrics and sandbox boundaries. A partial report remains
useful evidence but cannot promote a cell.

All available cells below are initially `planned`. Arch aarch64 is `unsupported`
because the official Arch Linux image index contains only amd64; Arch Linux ARM is
a separate distribution and is not silently substituted. AR-0702 owns native
kernel validation. Agent and workload ARs own promotion of their cross-product
cells after exact runtime evidence exists.

| Family | Pinned release/snapshot | libc baseline | amd64 image | arm64 image | Native kernel evidence |
| --- | --- | --- | --- | --- | --- |
| Ubuntu | 24.04.4 LTS | glibc 2.39 | planned | planned | x86_64 native-tested; aarch64 planned |
| Debian | 13.6 (trixie) | glibc 2.41 | planned | planned | planned |
| Fedora | 44 | glibc 2.43 | planned | planned | planned |
| Rocky Linux | 9.7 | glibc 2.34 | planned | planned | planned |
| AlmaLinux | 9.7 | glibc 2.34 | planned | planned | planned |
| openSUSE Leap | 16.0 | glibc 2.40 | planned | planned | planned |
| openSUSE Tumbleweed | 20260904 | glibc 2.44 | planned | planned | planned |
| Arch Linux | 20260830.0.582275 | glibc 2.44 | planned | unsupported | planned/unsupported |
| Alpine Linux | 3.24.1 | musl 1.2.6 | planned | planned | planned |
| openEuler | 24.03 LTS-SP2 | glibc 2.38 | planned | planned | planned |

The manifest retains both the human discovery tag and immutable OCI index digest.
It also pins the amd64 and arm64 child digests independently so an architecture
cannot be selected from a mutable tag or inferred from an index name.

## Agent package availability

Package presence is tracked separately from execution support. `package-inspected`
means registry metadata and the published archive were inspected; `metadata-only`
means an immutable package exists but its dependency/runtime closure was not proven.

| Agent | Pinned package | x86_64 | aarch64 | Compatibility boundary |
| --- | --- | --- | --- | --- |
| Codex | `@openai/codex@0.153.4` | package-inspected | package-inspected | Main binaries are static musl, but bundled `zsh` requires glibc 2.38 on both; arm64 `rg` also requires glibc 2.18. Alpine on both architectures and glibc baselines below 2.38 are unverified |
| OpenCode | `opencode-ai@1.18.29` | glibc and musl packages inspected | glibc and musl packages inspected | Inspected glibc binaries require at most glibc 2.17; no agent session ran |
| OpenDesk | `@bitclub.ai/opendesk-cli@0.3.5` | package-inspected | metadata-only | JavaScript requires Node.js 22.19 or newer; native transitive dependencies remain unverified |
| aider | `aider-chat==0.86.2` | metadata-only | metadata-only | Top-level wheel is universal and requires Python 3.10 through 3.12, but native transitive wheels and musl resolution remain unverified |

The manifests preserve npm SRI or PyPI SHA-256 integrity for every listed package
and variant. Package publication does not prove installation on any distribution.

## Reproducible inspection boundary

Registry indexes and child manifests were inspected with Docker buildx 0.36.1.
The pinned amd64 filesystem for each index was exported with crane 0.20.6 under
the configured development storage root. Release identity came from `os-release`;
glibc versions came from the shipped `libc.so.6`; Alpine musl 1.2.6-r2 came from
the image package database. npm and PyPI JSON metadata supplied package versions,
constraints and integrity, and selected npm archives were checked with `file` and
`readelf`. No foreign-architecture binary was executed.

The real external boundary exercised by AR-0701 is immutable registry and package
artifact retrieval. This establishes availability and contents only. Host cgroup
v2 delegation, systemd, SELinux/AppArmor, perf permission, PSI, BTF, native kernel
identity, agent startup and workload success all remain explicitly unverified.

## Native qualification boundary

The dedicated `native-platforms.yml` workflow runs on disposable native Ubuntu
x86_64 and aarch64 GitHub runners. It checks out the exact pull-request head,
runs argv-only bounded process and native-metrics tests, attempts the delegated
sandbox boundary in fail-closed mode, and uploads only canonical JSON. Command
and output digests are retained; raw logs, hostnames, environment contents and
filesystem paths are excluded. Hosted runners without user-systemd delegation
produce `native-functional-partial`, never a false `native-tested` result.

Native reports bind the exact manifest release to bounded operating-system
release evidence. Ubuntu uses the exact `VERSION` field, Debian 13.6 uses
`/etc/debian_version`, and openEuler uses `/etc/openEuler-release`; a major
version match alone is insufficient. Reports also retain whether AppArmor is
enabled and whether SELinux is enforcing, permissive, or disabled. If the
kernel LSM list or active SELinux state cannot be read, qualification is
partial rather than silently treating missing privilege as a disabled policy.
The sandbox test then proves behavior under the observed LSM state.

Sandbox prerequisites are platform-specific reviewed pins. The Ubuntu profile
uses bubblewrap `0.9.0-1ubuntu0.1`, systemd `255.4-1ubuntu8.17`, and
util-linux `2.39.3-9ubuntu6.6` from packages.ubuntu.com. The Debian 13.6
profile uses bubblewrap `0.12.0-1~deb13u1`, systemd
`257.13-1~deb13u1`, and util-linux `2.41.5-0+deb13u1` from
packages.debian.org. The openEuler 24.03 LTS-SP2 profile uses bubblewrap
`0.8.0-2.oe2403sp2`, systemd `255-43.oe2403sp2`, and util-linux
`2.39.1-22.oe2403sp2` from the official repo.openeuler.org source-package
index. Reports retain these public package identities and source URLs, and
validation rejects drift.

Virtual machines using the requested native instruction set may establish
functional behavior, but every report sets `performance_baseline` to false.
Containers and QEMU, UML or Bochs emulation are rejected as native evidence.
Performance claims require separately controlled native resources.

AR-0703 tracks the unavailable disposable booted Debian 13 and openEuler 24.03
LTS-SP2 x86_64/aarch64 lab cells. Container images, cross-compilation and emulation
do not substitute for those environments. Until that task provides genuine
capacity and all reports pass, the affected cells remain planned and AR-0702
remains incomplete.

To refresh a candidate, first inspect its mutable tag, then inspect/export the
resolved digest. Review all changes rather than replacing digests automatically:

```sh
docker buildx imagetools inspect IMAGE:TAG
docker buildx imagetools inspect --raw IMAGE:TAG
crane export IMAGE@sha256:DIGEST image.tar
python3 tools/platforms/validate_manifests.py
python3 -m unittest discover -s tests/platforms -p test_*.py
```

The failure fixtures prove that mutable image references, architecture alias
confusion and container-derived native claims are rejected.

## Local native x86 capacity

The [native x86 capacity contract](NATIVE_X86_CAPACITY.md) qualifies one explicitly
authorized existing Ubuntu x86_64 host as a bounded credential-free functional cell. Its
sanitized evidence is separate from the distribution support matrix: it does not promote a
platform or agent cell, establish an uncontended performance baseline, activate persistent
runner routing, or provide native aarch64 capacity. AR-0702 remains responsible for native
platform support claims.

## Emulated aarch64 portability lane

[The emulation manifest](../platforms/v1/emulated-aarch64.json) defines a
separate x86_64-hosted QEMU user-mode lane. It cross-builds the real workspace
for aarch64-unknown-linux-gnu, executes protocol, replay, adapter-logic and
bundle checks, and runs the packaged CLI. The emulator, cross linker, Rust
toolchain and Ubuntu aarch64 userspace are exact-version or digest pinned.
Commands have a 120-second bound, the workflow has a 20-minute bound, and its
temporary guest filesystem and build output are removed by the disposable
runner.

This lane is labeled only emulated-aarch64. QEMU user mode shares the booted
x86_64 host kernel; the recorded kernel release is host provenance, never guest
or native-aarch64 evidence. It does not qualify native hardware, native kernels,
timing, contention, architecture performance, distribution boot, Debian,
openEuler, or any cell owned by AR-0702/AR-0703. The closed evidence contract
rejects those claim elevations and unknown fields.

The binfmt interpreter receives the same digest-pinned guest userspace prefix as
the top-level QEMU runner, so nested aarch64 protocol and packaging executables
are exercised rather than silently replaced by host binaries. One mini-SWE unit
test deliberately launches a host fixture with invalid executable contents and
has architecture-dependent error classification; it remains a native-lane test
and is the only runtime exclusion. Runtime measurements from this lane are
diagnostic only and must not enter benchmark comparisons or support claims.
