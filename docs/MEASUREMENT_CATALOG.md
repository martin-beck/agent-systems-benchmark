# Measurement catalog v1

The measurement catalog is ASB's content-addressed inventory of optional, selectable collectors. It
is a data contract for frontends and automation; it does not define presentation, search,
selection interaction, or terminal behavior.

## Baseline scope

The v1 baseline contains exactly the metrics emitted by `asb-metrics` portable Linux procfs and
cgroup-v2 collectors. Every definition carries the exact runtime descriptor unit, aggregation,
scope, nominal `resolution_ns`, and a closed path-free source identity. The source identity maps to
the runtime descriptor spelling through `MeasurementSourceIdentity::runtime_descriptor`; callers
never supply a host path. Every published group contains at least one definition. Consumers must use
the catalog's `groups` and `measurements` arrays rather than constructing empty taxonomy groups from
the schema enum. Missing runtime evidence remains unavailable and is never converted to zero.

Several ASB outputs are deliberately not selectable catalog entries:

- provider usage and cost are mandatory attempt evidence represented by `NormalizedUsage` when an
  adapter can supply them;
- attempt outcome, completion latency, success proportion, quantiles, throughput, SLO decisions,
  comparison estimates, economics, and fairness are mandatory or derived analysis outputs;
- experiment, runner, replay, collector, and artifact provenance are identity/evidence fields, not
  numeric collectors.

Those outputs remain visible in results and reports, but a measurement picker must not offer them
as optional collection controls. Adding a genuinely selectable implementation requires a new
catalog definition, source-parity test, and compatible catalog/schema evolution.

Optional CSB monitoring is not baseline-qualified. No optional-CSB definition appears in the v1
baseline. If a later catalog carries one before qualification, both live and replay support must be
explicitly `unsupported` with reason `not_qualified`.

## Stability and validation

Catalogs are canonicalized by group enum order and measurement ID, bounded to seven groups and 128
definitions, and addressed by SHA-256 over the schema version and canonical content. IDs, public
ASCII text, names, units, sources, platform requirements, support modes, overhead, and evidence
limits are validated before use. Unknown fields fail deserialization. Duplicate IDs, case-folded
name collisions, incompatible units, empty advertised groups, noncanonical input, unsupported
versions, stale digests, and privacy-sensitive public text fail closed.

Untrusted JSON must enter through `MeasurementCatalogV1::from_slice_bounded` or
`MeasurementCatalogV1::from_reader_bounded`. Both enforce a 256 KiB wire ceiling before decoding;
the reader consumes at most the ceiling plus one byte. Constructor preflight rejects top-level and
nested count bounds and public text byte bounds before sorting, canonical serialization, or hashing.

The checked baseline and empty fixtures live in `crates/asb-protocol/fixtures/v1`; the generated
schema lives in `crates/asb-protocol/schema/v1/measurement-catalog.schema.json`.

## Selection contract and run-plan binding

`MeasurementSelectionV1` is the renderer-neutral choice object. It binds the exact catalog schema
and digest, a strictly ascending unique list of at most 128 stable IDs, live/replay mode, and a
bounded sampling interval. Its own domain-separated SHA-256 covers every field except the declared
selection digest. Empty selection is explicit and has no interval; a non-empty selection requires
an interval no faster than every selected definition permits and no longer than one hour. The
checked empty, one-measurement, and complete-catalog fixtures and
`measurement-selection.schema.json` are in the protocol v1 fixture and schema directories.

ASB plan schema v2 requires this object. Validation happens before result/work roots, durable
catalog mutation, workload preparation, or process launch. Catalog generation/digest, selection
digest, order, IDs, mode, qualification, platform features, privilege, authoritative target scope,
cadence, and a hard per-attempt evidence ceiling all fail closed. Plan schema v1 remains readable
through an explicit migration to its historical behavior: all seven process measurements at the
plan's process-poll interval. A v1 plan may not carry a v2 selection, and a v2 plan may not omit it.

The current runner owns an exact agent PID, so it accepts process-scoped definitions. It does not
yet own a verified delegated per-attempt cgroup; cgroup selections therefore fail with
`target_scope_unavailable`. The runner never substitutes its own cgroup. Selected procfs collection
gates the stat and IO source families independently before reads. Selection and catalog digests are
bound into v2 execution identity and exposed by plan/run evidence; terminal attempt evidence keeps
mandatory outcome/scoring fields while separately recording requested, collected, unavailable,
and omitted optional IDs. Search, group tri-state state, widgets, and every visible interaction
remain exclusively in the standalone `asb-tui` repository.

Independent frontends obtain the compiled baseline through the read-only control-v1
`measurement_catalog` operation. Its `1.2` publication wrapper adds only closed provenance and
content-addressed freshness; the nested catalog remains this exact contract. The runner advertises
the operation by selecting exact control version `1.2` and never requires a frontend to read an ASB installation path. See the
[frontend control API](FRONTEND_CONTROL_API.md#measurement-catalog-extension).

The immutable publication boundaries for the baseline catalog are recorded in
the [merge attestation](attestations/measurement-catalog-pr131-merge.json).
It binds both PR #131 and the first corrective PR #133 to their reviewed heads,
trees, twelve successful checks, and GitHub-verified merges. It accurately
preserves the original merge's missing-trailer classification and PR #133's
nonmatching author-name trailer classification; it does not rewrite history or
claim either merge was DCO-compliant.
