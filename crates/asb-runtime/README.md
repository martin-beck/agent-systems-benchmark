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
that precondition. Cgroup-backed AR-0103 containment is required for a hard
process-tree containment or deadline claim and before running untrusted code.

The lifecycle and fault cases are implementation tests against real Linux
processes, not a mechanical invariant or bounded proof. They assume Linux
process-group signalling, monotonic clock, and waitid WNOWAIT semantics. Formal
state and concurrency models and non-Linux execution remain deferred to their
dedicated assurance and portability tasks.
