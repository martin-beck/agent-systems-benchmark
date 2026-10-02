# Setup preflight and first-run selection

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
the credential environment name. It never stores an API-key value. Missing
authentication, signatures, and key management are visible development-only
warnings and do not block this prototype. Re-running the command edits the
same shared defaults; use `--no-default` when the selected agents should be
configured without replacing the global default set. Use `--json` for stable
automation output.

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
