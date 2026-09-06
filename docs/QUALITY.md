# Quality and platform assurance

The mandatory commands, pins, signature boundary and negative fixtures are
documented in [repository quality gates](QUALITY_GATES.md).

## Bootstrap gates implemented

The initial CI builds and tests Rust on native GitHub-hosted x86_64 and arm64
Ubuntu runners, checks formatting and Clippy with warnings denied, builds docs and
release binaries, and checks CLI exit behavior. All first-party Rust forbids unsafe
code and requires public documentation. Dependencies and the toolchain are pinned.

Coordination CI runs the reused fault tests with a 95% combined branch-aware
coverage floor, strict mypy, Ruff, JSON Schema and generated-state validation.
Public pull requests execute on disposable GitHub-hosted runners. Existing private
project runners are not assigned to this public repository.

The following gates are required implementation work, not claimed bootstrap features.

## Assurance roadmap and Agent Relay mapping

| Existing Agent Relay practice | ASB implementation target |
| --- | --- |
| Fatal compiler warnings, static analysis | Rust warnings/Clippy denied; strict module dependency boundaries |
| Provider ABI and conformance tests | Versioned protocol schemas, cargo-semver-checks and shared adapter contract suite |
| Property and concurrency tests | proptest, Loom for synchronization models and deterministic state-machine tests |
| Formal domain and recovery evidence | Kani proofs for bounded pure invariants; explicit proof bounds and exclusions |
| Fuzz and counterexample regression | cargo-fuzz on parsers, cassettes and path/archive handling; regressions in PR CI |
| Aggregate and critical coverage floors | cargo-llvm-cov: target 90% overall and 95% core/protocol/replay, with explicit denominators |
| Mutation/refactoring assurance | cargo-mutants on matching, SLO decisions and recovery; review surviving mutations |
| Dependency integrity/security | Cargo.lock, cargo-deny, RustSec/OSV audit, SBOM and artifact provenance |
| Shell/Python tooling tests | Minimal shell glue; ShellCheck/shfmt/Bats if introduced; existing Python strict gates |
| Workflow and secret audits | actionlint, zizmor, Gitleaks, pinned Actions and least permissions |
| Documentation consistency | rustdoc -D warnings, runnable examples, schema examples, local link/Markdown checks |
| Real production journeys | CLI -> real adapter -> replay endpoint -> workspace change -> independent grader |
| UI and artifact evidence | Stable JSON/exit codes, PTY cancellation and terminal snapshots where useful |
| Exact-tree publication | DCO trailers, SSH-signed commits, immutable-head CI and post-merge checks |

AR-0003 owns pinned installation of additional analyzers and their CI enforcement.
No empty test targets or placeholder always-green jobs count as implemented gates.
Miri checks applicable pure/unsafe-dependency boundaries; it does not model a real
kernel or substitute for native execution. Kani and Loom require separately pinned
toolchains. Keep proof/fuzz budgets bounded and preserve counterexamples.
Coverage is a regression constraint, not proof of correctness. Do not lower floors,
hide production code or refresh baselines just to pass.

## CI layers

- Every PR: deterministic formatting, types/lints, contracts, unit/property tests,
  credential-free integration, docs, schemas, DCO and supply-chain checks.
- Scheduled: bounded fuzz/mutation/model campaigns, distro containers, resource
  leak/endurance tests and dependency advisory refresh.
- Controlled native hosts/VMs: cgroup/PSI/perf correctness, real agent versions,
  full booted distribution kernels and performance measurements.
- Release: all required native matrix cells, clean package install/uninstall,
  reproducible rebuild comparison, checksums, SBOM, provenance and upgrade tests.

For public contributions, never run untrusted PR code on persistent trusted runners.
Do not use pull_request_target to execute a contributor checkout. Native privileged
jobs use disposable workers and a trusted immutable revision. Keep benchmark
resource reservations separate from build jobs and existing projects.

## Support target

| Distribution family | User-space tests | Native kernel validation |
| --- | --- | --- |
| Ubuntu LTS | Required x86_64 and aarch64 | Required both architectures |
| Debian stable | Required both | Required both |
| Fedora stable | Required both | Scheduled both |
| Rocky/AlmaLinux stable | Required both where images exist | Representative enterprise baseline |
| openSUSE Leap/Tumbleweed | Required both where images exist | Scheduled representative kernels |
| Arch Linux | x86_64; arm port tracked separately | No claim that Arch Linux ARM is official Arch |
| Alpine Linux | Required musl tests on both | Scheduled both |
| openEuler LTS | Required both | Required both; first-class support |

Exact releases, image digests, libc and package availability are pinned in the
[platform manifest](PLATFORMS.md). Test the
oldest supported glibc, static musl limits, cgroup v2 delegation, optional systemd,
SELinux/AppArmor, perf permissions and missing PSI/BTF gracefully. Every supported
agent x workload x distro x architecture combination needs an evidence status:
planned, build-only, simulated, native-tested or unsupported. A cross-build does
not establish native runtime or agent-package support.

## Licensing and authorship

Both repositories use MIT. Add SPDX-License-Identifier: MIT to source files.
Keep third-party notices and dataset licenses intact. Martin Beck's commits use
Martin Beck <martin.beck2@gmx.de> and exactly matching Linux-kernel-style
Signed-off-by trailers. Cryptographic SSH signing is an additional check.
Other contributors certify their own authorship; do not forge their sign-offs.

## Validation commands

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps
cargo build --locked --workspace --release
```
