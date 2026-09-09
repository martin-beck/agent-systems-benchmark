# Several agents with one provider

The CLI and TUI use the same fail-closed rule: select several compatible agent
identities and exactly one provider profile that covers all of them. The
provider profile, model, settings, and credential-reference digest are part of
the immutable selection identity.

## CLI route

```sh
asb provider-catalog > catalog.json
asb provider-plan --catalog-sha256 CATALOG_SHA256 \
  --provider-profile openai \
  --agent codex --agent opendesk \
  --credential-reference-sha256 CREDENTIAL_REFERENCE_SHA256 > selection.json
asb plan /absolute/path/EXPERIMENT.toml --provider-selection selection.json
asb run /absolute/path/EXPERIMENT.toml --provider-selection selection.json
```

Use the catalog digest returned by the same `provider-catalog` observation.
`codex` and `opendesk` are only an example: the catalog is authoritative for
the current compatible set. A stale catalog, duplicate agent, unsupported
adapter, or mismatched experiment fails before result/work roots or processes
are created.

## TUI route

Populate `MultiAgentCatalog` from the runner, then:

1. `search_agents` and `select_agents` (or `toggle_agent`) choose the set.
2. `compatible_providers` narrows the provider list to profiles covering every
   selected agent.
3. `select_provider` chooses one profile; `review` exposes the canonical plan
   and privacy-safe identity.
4. `dry_run` validates, then `confirm_plan` emits the same
   `ControlCall::CreatePlan` as the single-agent wizard.

The TUI must not silently fall back to a different provider. If users need
different profiles per agent, create separate explicit selections and plans;
do not describe those as one shared-provider run.

## Credentials and limits

Only a logical credential reference digest is transported. Credential values do
not belong in TOML, JSON selection files, argv, environment, logs, or reports.
OpenAI selection is supported when its preflight evidence is present; Ollama is
advertised only when its local-daemon evidence is verified. Unsupported
combinations remain unavailable rather than being guessed.
