# AR-1832 plan: explicit config end-to-end qualification

1. Start in a clean disposable project with no ASB user-home configuration and
   prove `asb setup --output CONFIG` is the only creation route.
2. Exercise interactive and noninteractive setup over supported agents,
   providers/models, workloads, monitor/measure selections, and bounded sweep
   dimensions using deterministic fakes.
3. Compile a nondefault named plan with `asb plan create --config CONFIG
   --output PLAN`; validate, run, sweep, report, and compare using the same
   explicit configuration path.
4. Run an opt-in, redacted development live smoke for every supported live
   adapter where credentials are available, while retaining fake qualification
   for CI and treating unavailable external services as typed evidence.
5. Prove legacy config/catalog/provider-plan commands and ProviderSelection
   artifacts cannot participate, and that no user-home state or secret value
   is created.
6. Obtain independent review, exact-head checks, signed protected merge, and
   exact-main post-merge evidence.
