# Native ARM64 and emulated AArch64 policy

ASB development does not require native ARM64 capacity. Missing, unavailable, failed, or
unauthorized native ARM64 hardware must not block implementation, pull-request integration,
release, documentation, or completion of an otherwise accepted AR.

The required AArch64 development gate is the pinned x86_64-hosted QEMU user-mode lane in
.github/workflows/emulated-aarch64.yml whenever the behavior can be tested without a native
kernel or architecture-specific performance facility. It covers:

- cross-compilation and release-binary startup;
- userspace protocol, replay, adapter, bundle, CLI, and TUI logic;
- portable error, cancellation, cleanup, and malformed-input paths; and
- architecture-specific dependency closure when the pinned guest userspace contains it.

The emulated workflow runs for pull requests and main. Its QEMU packages, cross toolchain, Rust
target, and ARM64 userspace image are pinned, and its temporary roots are deleted on every exit.
Required evidence is labeled emulated-aarch64.

Native x86_64 remains required where a feature needs real kernel, cgroup, PSI, perf, eBPF,
contention, timing, or performance evidence. QEMU shares the x86_64 host kernel and therefore
cannot prove:

- native ARM64 kernel, PMU, perf, eBPF, timing, contention, or performance behavior;
- a native ARM64 agent or workload support cell; or
- booted ARM64 Debian, openEuler, or another distribution kernel.

Those native ARM64 tests are optional future qualification. When run, they extend only the exact
support cell backed by immutable native evidence. Until then, the cell remains unqualified or
unsupported; no native claim is inferred from emulation.

## CI interpretation

Required pull-request and main gates are native x86_64 plus pinned QEMU AArch64 where applicable.
Native ARM64 jobs are optional manual or scheduled evidence and cannot hold a change or release
open. Historical native ARM64 results remain valid evidence for their exact revisions.

Support matrices must keep emulated-aarch64, native-tested, build-only, and unsupported distinct.
Performance results from QEMU never enter benchmark comparisons or baselines.
