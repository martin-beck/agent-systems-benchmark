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
asb easy record-campaign /absolute/path/MANIFEST.json --local-mock
```

The same guided surface exposes the catalog and evidence journeys without
requiring users to copy digests or reconstruct the underlying command shape:

```text
asb easy setup
asb easy provider-catalog
asb easy plan /absolute/path/EXPERIMENT.toml --use-config
asb easy report /absolute/path/RUN
asb easy compare /absolute/path/RUN_A /absolute/path/RUN_B
asb easy record /absolute/path/CAPTURE.json /absolute/path/CASSETTE.json --local-mock
asb easy replay /absolute/path/CASSETTE.json PROVIDER_PROFILE_SHA256 AGENT --local-mock
```

`easy replay` still requires the runtime-injected replay authority; a plain
CLI invocation fails closed rather than fabricating one. All local-mock routes
remain offline and delegate validation to the canonical ASB commands. Guided
setup accepts only the exact provider/model pairs advertised by
`provider-catalog`; workflow input and output paths must be absolute and may
not contain parent traversal components.

The guided command rejects live-provider, endpoint, and unknown options. It
never contacts a provider or creates a `LiveProviderAttempt`; ordinary and
live-provider paths retain their existing fail-closed behavior.
The campaign command seals the already-bounded capture matrix for strict
offline replay; it does not capture from or fall back to a provider.

Only public model/endpoint identities and digests are persisted. Missing,
stale, altered, or credential-bearing configuration is rejected before any
provider contact.

### Changing the provider, model, or agent set

The catalog and planner are the single selection authority. To change a
selection, refresh the catalog and generate a new content-pinned plan; do not
edit a prior selection or maintain a second model registry:

```sh
asb provider-catalog > catalog.json
asb provider-plan --catalog-sha256 "$(jq -r .catalog_sha256 catalog.json)" \
  --provider-profile openrouter \
  --agent codex --agent opendesk \
  --credential-reference-sha256 CREDENTIAL_REFERENCE_SHA256 > selection.json
```

Replace the provider profile and repeated `--agent` values with entries
advertised by the same catalog. The selected model is the catalog's exact
dated pin; stale, unsupported, mixed-provider, duplicate, or incompatible
agent selections fail closed and require generating a fresh plan. The planner
does not contact a provider or store a credential.

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

## Optional local OpenRouter measurement

The operator-only helper below is supplementary `remote_live` evidence; it is
not part of ASB runtime execution and is never invoked by CI. Supply one
bounded workload prompt on standard input. The helper reads the key from
`OPENROUTER_API_KEY` or the mode-0600 `~/.api_key_openrouter` file, and emits
only the workload identity, prompt digest, model identity, status, bounded
timings, and token counters. Prompts, responses, credentials, headers, and
private paths are not retained.

```sh
cargo run --locked -q -p asb-cli --bin asb -- provider-catalog > /tmp/asb-provider-catalog.json
python3 tools/local_openrouter_measurement.py \
  --catalog /tmp/asb-provider-catalog.json \
  --workload-id original.bug-fix --agent codex --trials 1
```

The helper loads the selected workload's checked-in `prompt.md` transiently;
the prompt and model response are never written. Repeat `--agent` for a
catalog-advertised set and pass `--model` only when it exactly matches the
fresh catalog profile. Unsupported, duplicate, stale, or mixed selections
fail before provider contact.

This helper does not make `asb run` or `asb sweep` live-provider capable. Those
commands retain their runtime-owned authority boundary and deterministic local
mock path; remote reachability is optional evidence only.
