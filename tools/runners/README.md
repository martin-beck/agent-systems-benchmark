# Development-host runner lifecycle

These scripts prepare one isolated, pseudonymous GitHub Actions runner beneath a canonical
`ASB_STORAGE_ROOT` named `asb-runner-storage-*` beneath `/srv/data/projects`. A root-owned
operator broker owns the runner root,
complete installation manifest, lease, registration state and retained diagnostic summaries. A
distinct dedicated service identity can traverse and list the runner root but owns only `_work`,
`_diag`, cache, artifacts and temporary roots; the operator-only control directory remains mode
0700. The service account must have a non-interactive shell. Launch makes its configured account home
inaccessible and supplies fresh HOME, XDG and temporary directories inside the resettable root. The
account has only its primary group, and one storage parent contains exactly one runner.
Every immutable installation entry is checked by type, owner, mode, link target and SHA-256 before
launch. The archive is copied into operator-only storage before its pinned digest is checked and
extracted, preventing source-path replacement. `health.sh` and `reset.sh` require a current
mode-0600 lease and exact versioned label.

Registration is an operator-only boundary because GitHub registration material is secret. After
`setup.sh`, an authorized operator requests a short-lived token and pipes it directly to `register.sh`;
the dedicated service identity never receives the operator's long-lived GitHub credentials. The helper
invokes the pinned runner with `--unattended --ephemeral --disableupdate --no-default-labels`, the
single indivisible label from `common.sh`, and the pseudonymous runner name. Never enable tracing,
persist, or echo the token. Registration refuses to run while the service identity owns any process,
bounds and validates the one-line token, requires a procfs mount that hides operator processes from
the service identity, clears the token immediately, and makes generated listener state root-owned and
non-writable by the service identity. The one-time registration token is never stored.
The listener needs read access to its current ephemeral authentication state. The three logical
principals are the root operator, the pinned listener under its dedicated service identity, and
untrusted workflow steps in a separate job container that cannot mount the installation or control
roots.

`setup.sh` writes `ACTIONS_RUNNER_REQUIRE_JOB_CONTAINER=true` into the immutable runner environment.
`launch.sh` re-verifies every boundary and starts one job through a bounded systemd service with
`ProtectSystem=strict`, an explicit mutable-path allowlist, `NoNewPrivileges`, and control-group
termination. GitHub documents that this variable rejects jobs without a declared job container.
No persistent-runner workflow may dispatch until it uses an independently reviewed digest-pinned
container and policy rejects host-path mounts. The current manual workflows intentionally fail closed
at this boundary until that integration is complete.

The operator holds one lifecycle lock across registration, launch, diagnostic collection and reset.
Launch invokes the verified `Runner.Listener` directly because the upstream convenience `run.sh`
rewrites a helper inside the otherwise immutable installation. Reset removes all ephemeral
registration state, so every subsequent run requires a fresh short-lived registration.
Failed registration removes partial credential state before releasing the lifecycle lock.

The service must use the dedicated service identity, start only while its lease is current, and execute
`launch.sh`, never `run.sh` directly. Before every start, run `health.sh`; after every stop, confirm
the listener and descendants are absent before collecting diagnostics and running `reset.sh`. Reset
holds a nonblocking lock, refuses while any dedicated-service process remains, and atomically quarantines
each mutable tree before bounded same-filesystem removal. Reset requires a completed private
diagnostic summary, recovers one bounded interrupted quarantine set, and destroys the raw diagnostic
tree. `collect-diagnostics.sh` accepts only a
bounded flat regular-file set and retains only indexed byte counts and SHA-256 values in the
operator-only control root; raw names, paths and contents are never retained or uploaded.

The boundary assumes root and the host kernel/system manager are trusted. It does not claim that a
container defeats a hostile kernel or a workflow allowed to request host mounts. Interruption safety
comes from the transient service control group plus the mandatory idle check before diagnostics or
reset; reboot persistence remains outside this AR.

AR-0830 permits only a protected manual prequalification canary. Ordinary workflow routing remains
disabled until AR-0832 qualifies this capacity and a separately reviewed activation enables it.
