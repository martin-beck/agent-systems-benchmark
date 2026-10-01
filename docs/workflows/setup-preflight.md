# Setup preflight

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
