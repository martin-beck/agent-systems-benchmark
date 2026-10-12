# AR-1826 plan: explicit setup configuration and plan-binding contract

1. Define one versioned, portable, secret-free configuration schema written
   only to a path explicitly supplied to `asb setup --output`.
2. Specify configuration-relative path resolution, atomic creation, explicit
   parent-directory notice/creation, overwrite behavior, migrations, and
   precise failures for absent, unreadable, malformed, or unsafe paths.
3. Model the complete selected benchmark: agents and tool identities,
   provider/adapters, credential references only, models, workloads,
   monitors/measures, run/sweep bounds, execution roots, live/network/cost
   acknowledgement, and internal registry snapshot identities.
4. Define plan binding: `asb plan create --config CONFIG --output PLAN` embeds
   the configuration digest and canonical config identity; every later plan,
   run, and sweep validation receives `--config CONFIG` and rejects mismatch.
5. Specify that no API-key bytes, credential values, home-directory defaults,
   ProviderSelection document, provider-selection path, or catalog-hash CLI
   ceremony may be needed by the normal route.
6. Add schema/round-trip/redaction/relative-path/atomic-write compatibility
   tests and update the command contract documentation before downstream work.
