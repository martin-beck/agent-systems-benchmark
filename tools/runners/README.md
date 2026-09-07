# Development-host runner lifecycle

These scripts prepare one isolated, pseudonymous GitHub Actions runner beneath a canonical
`ASB_STORAGE_ROOT` in `/srv/data/projects`. `setup.sh` verifies the pinned upstream archive and
creates private installation, cache, artifact, temporary, and control roots. `health.sh` and
`reset.sh` require a current mode-0600 lease and the exact versioned label set.

Registration is an operator-only boundary because GitHub registration material is secret. After
`setup.sh`, an authorized operator runs the pinned runner's `config.sh` with all of
`--unattended --ephemeral --disableupdate --no-default-labels`, the exact labels printed by
`common.sh`, the pseudonymous runner name, and a short-lived token supplied outside the repository
and captured logs. Never persist or echo the token. The resulting `.runner` file must remain private
and must not be published as evidence.

The service must use a dedicated least-privilege identity, start only while its lease is current,
and execute the pinned `run.sh` from the dedicated runner root. Before every start, run `health.sh`;
after every stop, confirm the listener and descendants are absent before running `reset.sh`. Reset
holds a nonblocking lock and atomically quarantines each mutable tree before replacement and bounded
same-filesystem removal. A same-identity hostile process can still race directory ancestors, so the
service identity must not be shared with jobs or another runner.

AR-0830 permits only a protected manual prequalification canary. Ordinary workflow routing remains
disabled until AR-0832 qualifies this capacity and a separately reviewed activation enables it.
