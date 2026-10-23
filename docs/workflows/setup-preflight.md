# Setup preflight and explicit configuration

The migration route is explicit-file only. A setup invocation that creates
durable configuration must receive `--output CONFIG`; no XDG or home-directory
file is an authority for this route:

```sh
asb setup --output CONFIG
asb plan create --config CONFIG --output PLAN --workload WORKLOAD
asb plan validate --config CONFIG PLAN
asb run --config CONFIG PLAN
asb sweep --config CONFIG PLAN
```

The versioned `explicit-setup-config-v1` document contains the selected agents,
adapters/providers, digest-only credential references, models, workloads,
monitors/measures, bounded run/sweep policy, relative execution root, live/
network/cost acknowledgements, and immutable registry snapshot digests. It is
portable and secret-free: API-key bytes, provider-selection documents, catalog
hash command ceremony, private paths, and implicit defaults are not fields in
this contract. Parent directories may be created only after the explicit path
is supplied; writes are validated and atomic, and existing files are never
silently overwritten.

Each plan embeds `config_id` and the SHA-256 digest of canonical configuration
bytes. `plan validate`, `run`, and `sweep` require the same explicit `--config`
path and reject either a changed document or a different identity before any
execution. The checked-in schemas are
[`explicit-setup-config-v1`](../../crates/asb-config/schema/v1/explicit-setup-config.schema.json)
and [`config-bound-plan-v1`](../../crates/asb-config/schema/v1/config-bound-plan.schema.json).

The older provider-selection examples below describe the pre-migration
development surface and are retained only as historical context for downstream
ARs; they are not authority for the explicit route.

`asb setup` is a credential-free setup wizard contract. Human-readable output
is the default; add `--json` when a frontend needs the versioned response.
Running it without selection arguments is a side-effect-free catalog and
authentication checklist:

```text
asb setup
asb setup --json
```

To persist one shared provider/model selection for several coding agents, pass
each agent explicitly. The configuration is written atomically to the normal
ASB XDG configuration store; `--config /absolute/path/config.json` is available
for disposable tests and runners:

```text
asb setup --agent opencode --agent opendesk \
  --provider-profile openrouter \
  --model cohere/north-mini-code:free \
  --credential-env OPENROUTER_API_KEY \
  --persist
```

The persisted document contains only provider/model identities and a digest of
the credential environment name. The environment name must be the provider's
runtime channel (`OPENROUTER_API_KEY` for OpenRouter or `OPENAI_API_KEY` for
OpenAI), so a saved selection cannot point at a name the adapter will ignore.
It never stores an API-key value. Missing
authentication, signatures, and key management are visible development-only
warnings and do not block this prototype. Re-running the command edits the
same shared defaults; use `--no-default` when the selected agents should be
configured without replacing the global default set. Use `--json` for stable
automation output.

Only providers with a currently qualified ASB runtime route can be persisted by
the multi-agent setup path. Gemini and Ollama remain visible in the read-only
catalog until their agent/runtime contracts are qualified; selecting them does
not create a partial configuration.

`asb setup` is a bounded, side-effect-free first step for both interactive and
automated configuration. It emits versioned JSON and never contacts a provider.

```sh
asb setup --format=json
asb setup --provider-profile openai --model gpt-4o-mini --output setup.json
```

Provider and model are validated as a pair. The `--output` form writes the
complete contract atomically only after validation; invalid or incomplete input
does not create or replace a file. Credentials are never accepted by this
command. See the checked-in [output schema](../../crates/asb-cli/schema/v1/setup-output.schema.json).

The durable provider/model document is maintained by `asb-config`. Callers
construct a [`ProviderSelection`](../../crates/asb-config/src/lib.rs) and pass
the returned document to `ConfigStore::save`; the mutation validates the
provider, connection, agent references, and shared-default propagation before
any bytes are written. Configuration files are owner-private, redacted to
public identities and digest-only credential locators, and installed through a
sync-and-rename transaction. A bounded interrupted temporary file is recovered
on the next save. Missing development authentication, signatures, or key
management remains a visible warning and does not block setup or offline
qualification; live production authorization remains fail-closed.
