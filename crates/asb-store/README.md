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

## Optional trace projection

`TraceExporter` maps validated vendor-neutral ASB causal spans to bounded OTLP
JSON records. The mapping pins OpenTelemetry GenAI semantic conventions commit
`b5d8440f6f126738fd50f927752cd669772c517b` and its development schema URL
`https://opentelemetry.io/schemas/gen-ai-dev/1.42.0-dev`. The exporter performs
no I/O: a transport drains its bounded queue, while contention or saturation
drops telemetry rather than delaying or corrupting a run.

Raw prompts, responses, error messages, endpoints, and credentials are not
representable. Content evidence is limited to an optional digest and byte count
and is excluded unless explicitly enabled.
