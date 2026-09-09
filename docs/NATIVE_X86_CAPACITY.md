# Native x86 qualification capacity

ASB can use one explicitly authorized, existing Ubuntu 24.04 x86_64 development host as a
credential-free native functional qualification cell. The local operator invokes
`tools/platforms/native_x86_capacity.py`; it is not an automatic GitHub Actions route.

The tool requires a clean immutable source descendant, exact reviewed distribution, kernel and
package versions, native x86_64 execution, unified cgroup v2, CPU/memory/I/O PSI, and a private
same-filesystem cell root beneath `/srv/data/projects`. It acquires one nonblocking ASB lease and
runs the fixed process, sandbox, metrics and workload checks in collected transient user services.
Each service has a private network namespace, no new privileges, a strict read-only host view,
private temporary state, bounded CPU, memory, tasks and runtime, and an external target directory
inside a size-capped temporary filesystem in the disposable cell. Cargo is locked and offline.

Only canonical JSON evidence is retained. It contains public distribution and kernel provenance,
exact package manifests and binary hashes, immutable source identity, resource ceilings,
argv/output digests and cleanup results. The unreadable root-owned boot image is bound to its exact
package manifest and the running kernel is independently bound by its notes and version digests; no
claim of a directly hashed boot image is made. Installed package copyright-file digests and the
official Ubuntu updates repository identify the reviewed provenance/license material without
redistributing those tools. Evidence never stores command output, environment
values, hostnames, addresses, account
identifiers, credentials, prompts, responses, private paths or raw logs. The cell tree and transient
units are removed before evidence can be emitted. A concurrent or stale cell, unsafe path, changed
source, unsupported host identity, missing capability, failed check, excessive output, timeout or
cleanup residue fails closed.

This evidence is native functional evidence only. It is not a performance baseline: the lease
excludes another invocation of this ASB harness but cannot exclude unrelated host activity. The
existing host has no separate provider spend, while its operational cost is unmeasured. Capacity is
a single local cell with no availability SLA. It is not eligible for public pull-request code and
does not activate the separately controlled persistent-runner workflows.

Native aarch64 remains unavailable and unsupported by this capacity. Emulation, cross-building and
archive inspection never upgrade that status. AppArmor registration is observed, but enforcement is
not claimed without a denial oracle; SELinux is unavailable on the qualified host.

The committed [Ubuntu native functional report](../platforms/v1/native-x86-evidence/ubuntu-24-04-x86-64-native-functional.json)
binds the successful disposable run to its exact source commit and tree. The adjacent
`native-x86-capacity.schema.json` is the closed evidence contract.

## Rolling hosted runners are not native qualification

The `ubuntu-24.04` GitHub-hosted label is rolling. Its patch release can advance independently of
the exact Ubuntu 24.04.4 identity above. The native validator continues to reject every different
patch release, including Ubuntu 24.04.5; hosted labels are never treated as aliases for the
reviewed release.

The native-platform workflow first classifies the public distribution identity. On the exact
reviewed release it invokes the unchanged native evidence collector and retains an artifact only
after that collector succeeds. On a newer well-formed Noble 24.04 patch release it instead runs the
same bounded process, metrics and sandbox functional checks and emits `hosted-portability`
evidence under its distinct closed schema. That document is explicitly
`functional-portability-only`, is not a performance baseline, and cannot become or overwrite a
native qualification artifact. Any other distribution family, malformed identity, failed check,
unsafe output path or partial evidence fails before artifact retention.

The hosted projection contains only the runner label, public distribution and architecture,
immutable source commit/tree, and argv/output digests. It excludes command output, raw diagnostics,
environment values, hostnames, account identifiers, credentials and private paths. Pinned QEMU
AArch64 remains the required portable AArch64 lane; genuine native AArch64 remains optional and
unqualified without separate immutable evidence. A future Ubuntu 24.04.5 native qualification
requires a separate reviewed pin update and new evidence rather than reinterpretation of the
historical Ubuntu 24.04.4 result.
