# ASB process runtime

`asb-runtime` owns trusted native subprocess execution. Each child starts as a
new process-group leader. The runtime drains stdout and stderr concurrently,
retains only configured byte limits, applies a monotonic deadline, terminates
the complete process group, and reaps its direct child.

Leader exit is observed with `waitid(WNOWAIT)`. Keeping the leader waitable
fences its PID and process-group identity until descendants have been cleaned
up, preventing a stale handle from signalling an unrelated reused ID. A second
cancellation is a no-op.

This is not an isolation boundary. Only trusted programs belong in the native
backend; namespaces, cgroups, untrusted-code containment, resource leases, and
cross-platform backends are owned by later runtime tasks.

The retained stdout and stderr prefixes are bounded, while reader threads keep
draining discarded suffixes to prevent pipe deadlock. Stdin remains owned by
the caller. A trusted descendant that deliberately creates a different session
or process group can leave this ownership boundary; cgroup-backed containment
is required before running untrusted code.

The lifecycle and fault cases are implementation tests against real Linux
processes, not a mechanical invariant or bounded proof. They assume Linux
process-group signalling, monotonic clock, and waitid WNOWAIT semantics. Formal
state and concurrency models and non-Linux execution remain deferred to their
dedicated assurance and portability tasks.
