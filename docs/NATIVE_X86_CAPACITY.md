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
