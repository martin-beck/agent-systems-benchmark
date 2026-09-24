# One benchmark run with shared agent configuration

After the offline readiness check, this route validates one bounded benchmark
run and applies the same content-pinned configuration to every selected agent.
The examples use synthetic IDs and repository-relative paths. They are syntax
fixtures: CI validates them without launching a benchmark, agent, provider, or
network connection.

1. Select all agents before planning. The provider selection is atomic: if one
   selected agent is incompatible, no agent receives a partial configuration.

   ```text
   asb provider-plan --catalog-sha256 CATALOG_SHA256 \
     --provider-profile synthetic \
     --credential-reference-sha256 CREDENTIAL_REFERENCE_SHA256 \
     --agent codex --agent opendesk
   ```

2. Validate the experiment and selection, then run exactly one point:

   ```text
   asb plan benchmark-experiment.toml --provider-selection benchmark-selection.json
   asb run benchmark-experiment.toml --provider-selection benchmark-selection.json
   asb report results/runs/synthetic-run
   ```

   A terminal result identifies the workload revision, measurement/configuration
   digest, selected agents, and bounded manifest/journal artifacts. A successful
   syntax check does not claim that a benchmark ran.

3. If an agent is incompatible, expect a structured `incompatible_agent`
   failure with `applied_agents: []`. Do not retry with a subset and call it the
   same run; repair the shared configuration and create a new run ID.

The executable contract is [`benchmark-run-shared-config-v1.json`](../../tools/tutorials/benchmark-run-shared-config-v1.json),
and the result fixtures are under `tools/tutorials/fixtures/v1/`. Validate the
tutorial offline with:

```sh
python3 tools/tutorials/validate.py tools/tutorials/benchmark-run-shared-config-v1.json
```
