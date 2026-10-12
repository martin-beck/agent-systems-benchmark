# AR-1830 plan: config-bound validation, run, and sweep

1. Require `--config CONFIG` for `asb plan PLAN`, `asb run PLAN`, and
   `asb sweep PLAN`; reject absent config, digest/schema/path mismatch, stale
   registry resolution, and config modified after plan compilation with a
   recompile command.
2. Revalidate provider/model/agent compatibility, tool/workload readiness,
   monitor/measure requirements, safe result/work roots, live acknowledgement,
   credential-reference availability, sweep cardinality, concurrency, timeout,
   cost, and storage bounds before creating execution artifacts.
3. Use config + plan only to construct each deterministic point. Live mode may
   resolve a credential reference immediately before dispatch but must never
   persist it, and may never fall back to mock, replay, or another provider.
4. Expand selected agents × models × workloads × repetitions deterministically,
   report the bounded point count, preserve stable point identities, support
   safe resume, and retain per-point monitor/measure provenance.
5. Add fake-provider and opt-in live-development integration coverage for
   success, invalid credential reference, stale model, unsupported tuple,
   cancellation, partial sweep, resumption, and redaction paths.
