# ASB CSB execution boundary

`asb-csb-runner` is the opt-in, versioned boundary between ASB and one
content-pinned CSB executable. It does not link CSB, Python, benchmark
generators, or syzkaller into the default ASB runtime.

The boundary accepts only the `external_application` execution mode, relative
artifact paths, an allowlisted environment, bounded arguments, and SHA-256
identities for the executable and source tree. Negotiation uses the existing
ASB JSON-RPC v1 framing contract. Shell commands, network access, ambient
credentials, plugins, generators, and syzkaller coupling are rejected.

Execution uses the AR-0103 sandbox and resource-lease boundary after verifying
the pinned executable bytes. A durable `Running` intent precedes spawn and a
durable `Collecting` transition precedes artifact collection. If spawn,
collection, or cleanup cannot be proved, recovery reports
`NeedsReconciliation`; callers must not retry an uncertain effect.

Native qualification is intentionally narrow: the pinned standard-library-only
`bm-external/bwrap/bm-bwrap.py` external-application fixture is exercised on
Ubuntu 24.04 x86_64 with Bubblewrap 0.9.0, systemd 255, and util-linux 2.39.3.
The executable handoff uses the verified open file descriptor, so changing the
workspace pathname after verification cannot change the executed inode. The
tests also pin `/usr/bin/python3.12` 3.12.3 by SHA-256 and cover successful
execution, explicit cancellation, deadline escalation, network denial, and
zero residual CPU leases. They do not qualify
CSB's Python dependency graph, generators, monitors, nested schedulers,
syzkaller integration, aarch64, musl, non-Linux systems, or other tool versions.

Workspace and state roots must be absolute, existing, disjoint directories
whose lexical ancestry contains no symlink. Hostile same-UID replacement of an
ancestor directory remains outside this boundary; native sandbox deployment
must place those roots below a trusted private parent.
