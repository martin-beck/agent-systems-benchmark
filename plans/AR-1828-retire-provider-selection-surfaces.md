# AR-1828 plan: retire provider-selection surfaces

1. Inventory every public command, flag, schema, docs example, test fixture,
   TUI bridge, and runtime adapter use of `asb config`, `provider-catalog`,
   `provider-plan`, `ProviderSelection`, `--provider-selection`, catalog hash
   CLI input, and XDG/user-home selection persistence.
2. Move required provider/model discovery and compatibility functionality
   behind setup and config validation APIs; do not expose a replacement manual
   selection artifact.
3. Remove the commands, types, files, schemas, help/completion entries, flags,
   migrations, and tests. Old invocations fail with a typed migration message
   pointing to `asb setup --output CONFIG`.
4. Ensure TUI and programmatic callers invoke setup/config APIs rather than
   reconstructing legacy selection documents.
5. Add negative tests proving no normal CLI route reads implicit home config or
   accepts a provider-selection document, while internal registry resolution
   remains bounded and secret-free.
