# AR-1829 plan: config-bound plan compilation

1. Require both `--config CONFIG` and `--output PLAN` on `asb plan create`.
   Do not select a default filename, default plan directory, or implicit config.
2. Announce and safely create a missing output parent only after validation;
   reject existing output unless `--overwrite` is explicit and preserve existing
   plans on every validation failure.
3. Resolve the setup configuration into exact agent/provider/model/workload/
   monitor/measure bindings and bounded sweep dimensions. Validate tool and
   workload readiness before writing a plan.
4. Embed config schema/version, digest, canonical identity, registry snapshot
   identities, selected revision identities, resolved safe paths, and computed
   sweep cardinality. Persist no credential value.
5. Remove legacy plan-create flags that duplicate selection authority; emit a
   precise remediation if callers try them.
6. Add deterministic plan compilation, overwrite, path, stale-resolution,
   malformed-config, and no-secret tests.
