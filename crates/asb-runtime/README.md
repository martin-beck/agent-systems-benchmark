# ASB process runtime

`asb-runtime` owns trusted native subprocess execution. Each child starts as a
new process-group leader. The runtime drains stdout and stderr concurrently,
retains only configured byte limits, applies a monotonic deadline to the owned
process group, terminates that group, and reaps its direct child.

Leader exit is observed with `waitid(WNOWAIT)`. Keeping the leader waitable
fences its PID and process-group identity until descendants have been cleaned
up, preventing a stale handle from signalling an unrelated reused ID. A second
cancellation is a no-op.

This is not an isolation boundary. Only trusted programs belong in the native
backend; namespaces, cgroups, untrusted-code containment, resource leases, and
cross-platform backends are owned by later runtime tasks.

The retained stdout and stderr prefixes are bounded, while reader threads keep
draining discarded suffixes to prevent pipe deadlock. Stdin remains owned by
the caller. Native commands must not daemonize, call `setsid` or `setpgid`, or
otherwise let a descendant escape the owned process group while retaining an
inherited output pipe. Such an escape can keep `wait` blocked after the runtime
has killed its owned group, so the timeout and cleanup guarantee depends on
that precondition. The cgroup-backed sandbox below supplies the stronger
containment required before running untrusted code, subject to its documented
Linux delegation and kernel-boundary limits.

The lifecycle and fault cases are implementation tests against real Linux
processes, not a mechanical invariant or bounded proof. They assume Linux
process-group signalling, monotonic clock, and waitid WNOWAIT semantics. Formal
state and concurrency models and non-Linux execution remain deferred to their
dedicated assurance and portability tasks.

## Rootless sandbox backend

The sandbox module composes exact-version-pinned Bubblewrap with a delegated
systemd user scope. Bubblewrap supplies user, mount, PID, IPC, UTS and network
namespaces, clears the environment, drops capabilities, disables nested user
namespaces, exposes only read-only runtime directories plus the writable
workspace and temporary filesystem, and denies host networking. The systemd
scope applies memory, swap, task, CPU-bandwidth and monotonic runtime limits.
Exact-pinned taskset enforces CPU placement even when a user slice lacks
effective cpuset delegation.

CPU lease files in one shared lease root make benchmark and CI reservations
mutually exclusive. Their root is a trusted coordinator directory. Internally
generated kernel-random scope names are not caller-selectable, and spawn returns
only after a nonce in the new cgroup proves ownership. A collision therefore
cannot authorize cleanup of an existing scope. A crash, unproven ownership or
unconfirmed scope cleanup leaves conservative stale leases requiring
process/cgroup reconciliation; Drop retries cleanup even after the launcher is
terminal. The backend rejects absent or version-mismatched tools, unavailable
delegation, host network requests, invalid paths and unbounded inputs.

This requires Linux, cgroup v2, a per-user systemd manager and Bubblewrap with
user-namespace support. It is not a VM, does not hide CPU/kernel identity, and
does not protect against kernel vulnerabilities. Support remains limited to
exact natively tested tool, kernel, distribution and architecture combinations.
