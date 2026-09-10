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

The immutable publication boundary for the baseline catalog is recorded in the
[PR #131 merge attestation](attestations/measurement-catalog-pr131-merge.json).
That attestation accurately preserves the original merge commit's missing-DCO
classification; it does not rewrite history or claim that commit was compliant.
