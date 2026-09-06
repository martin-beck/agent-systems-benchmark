# ASB durable store

`asb-store` persists immutable run manifests, checksummed transition journals,
and content-addressed artifacts. Writes are staged beside their destination,
synced, renamed, and followed by a parent-directory sync. Per-store file
locking serializes writers from independent processes.

Recovery is conservative: a journal ending in `running` or `collecting`
returns `needs_reconciliation`; it never authorizes another execution. Invalid
paths, unknown schema versions, broken transition order, checksum mismatches,
truncation, and configured size limits fail closed.

The store records durable intent and evidence. Inspecting live processes,
cgroups, and remote effects before resolving an uncertain attempt belongs to
the runtime/coordinator layer. Filesystem durability still depends on the
mounted filesystem honoring file and directory `fsync` plus atomic same-volume
rename.
