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
