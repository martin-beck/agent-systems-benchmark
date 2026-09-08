# ASB durable store

`asb-store` persists immutable run manifests, checksummed transition journals,
and content-addressed artifacts. Writes are staged beside their destination,
synced, renamed, and followed by a parent-directory sync. Per-store file
locking serializes writers from independent processes.

Recovery is conservative: a journal ending in `running` or `collecting`
returns `needs_reconciliation`; it never authorizes another execution. Invalid
paths, unknown schema versions, broken transition order, checksum mismatches,
truncation, and configured size limits fail closed.

Verifier observations are committed once per run after independently re-hashing
the referenced environment and optional submission artifacts. They preserve ready, task-failed,
patch-failed, timed-out, and environment-failed outcomes as distinct durable
facts; only ready observations can enter offline scoring. Loading repeats both
the observation self-digest check and the referenced-artifact check.

Store paths are private and final file and directory opens reject symbolic
links. The fault suite deterministically injects partial-write/disk-full-like
and rename-boundary failures; it is not evidence from a real ENOSPC filesystem.
The store assumes its private root is not concurrently renamed or replaced by
another process with the same operating-system identity; defending against
hostile replacement of ancestor directories requires a future directory-fd
resolution layer.

The store records durable intent and evidence. Inspecting live processes,
cgroups, and remote effects before resolving an uncertain attempt belongs to
the runtime/coordinator layer. Filesystem durability still depends on the
mounted filesystem honoring file and directory `fsync` plus atomic same-volume
rename.
