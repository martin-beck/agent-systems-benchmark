# Several agents with one provider

The CLI and TUI use the same fail-closed rule: select several compatible agent
identities and exactly one provider profile that covers all of them. The
provider profile, model, settings, and credential-reference digest are part of
the immutable selection identity.

## CLI route

```sh
asb provider-catalog > catalog.json
asb provider-plan --catalog-sha256 CATALOG_SHA256 \
  --provider-profile openai|openrouter \
  --agent codex --agent opendesk \
  --credential-reference-sha256 CREDENTIAL_REFERENCE_SHA256 > selection.json
asb plan /absolute/path/EXPERIMENT.toml --provider-selection selection.json
asb run /absolute/path/EXPERIMENT.toml --provider-selection selection.json
```

Use the catalog digest returned by the same `provider-catalog` observation.
`codex` and `opendesk` are only an example: the catalog is authoritative for
the current compatible set. `openrouter` selects the pinned credential-free
OpenRouter profile (same free model for every compatible adapter, routed
through the OpenRouter public API with `OPENROUTER_API_KEY` as the environment
credential reference); `openai` selects the pinned OpenAI profile. A stale
catalog, duplicate agent, unsupported adapter, or mismatched experiment fails
before result/work roots or processes are created.

## Per-user OpenRouter configuration

Persist the pinned dated free-model selection and credential-free
`OPENROUTER_API_KEY` environment reference, then use it without repeating the
digest on every command:

```sh
asb config openrouter
asb provider-plan --catalog-sha256 CATALOG_SHA256 --use-config --agent codex > selection.json
asb plan /absolute/path/EXPERIMENT.toml --use-config
asb run /absolute/path/EXPERIMENT.toml --use-config
asb sweep /absolute/path/EXPERIMENT.toml --use-config
```

For deterministic offline qualification, the guided wrapper requires an
explicit local fixture marker and delegates to the same config-bound paths:

```text
asb easy run /absolute/path/EXPERIMENT.toml --use-config --local-mock
asb easy sweep /absolute/path/EXPERIMENT.toml --use-config --local-mock
```

The guided command rejects live-provider, endpoint, and unknown options. It
never contacts a provider or creates a `LiveProviderAttempt`; ordinary and
live-provider paths retain their existing fail-closed behavior.

Only public model/endpoint identities and digests are persisted. Missing,
stale, altered, or credential-bearing configuration is rejected before any
provider contact.

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
OpenAI and OpenRouter selections are supported when their preflight evidence is
present; Ollama is advertised only when its local-daemon evidence is verified.
Unsupported combinations remain unavailable rather than being guessed.
