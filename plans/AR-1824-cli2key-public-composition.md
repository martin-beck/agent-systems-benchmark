# AR-1824 plan: cli2key public provider composition

1. Define the installed cli2key catalog/receipt projection needed by provider-plan and provider-selection without embedding credentials, paths, endpoints, or provider bodies.
2. Add a closed cli2key provider-plan path for Codex Responses with exact model, catalog, bridge, executable, endpoint, and credential-reference digests.
3. Compose the verified project-local receipt with the runtime-owned sidecar authority for direct run and bounded sweep; callers cannot provide endpoint, key, executable, or fallback provider values.
4. Add credential-free fake tests for install/select/run/sweep/report/compare, malformed/stale/substituted receipts, teardown and no-fallback behavior.
5. Run an opt-in live development probe only if locally authorized; retain redacted typed evidence and never make it a CI/release requirement.
6. Obtain independent review, signed DCO PR, exact-head CI, protected merge, exact-main post-merge verification, and durable acceptance evidence.
