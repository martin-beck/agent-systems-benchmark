# End-to-end benchmark workflow

This guide describes the credential-free development path and the separately
supervised live-provider path. Both use the same digest-bound catalog and plan
contracts; only live execution may contact a provider.

## Offline/local-mock qualification

Build and inspect the CLI, then exercise the bounded workflow with local fixtures:

```sh
cargo build --locked --workspace
cargo run --locked -p asb-cli -- provider-catalog
cargo run --locked -p asb-cli -- workload-catalog
cargo test --locked --workspace -- --test-threads=1
```

The local/mock and strict-replay paths require no credentials, provider traffic,
dataset download, or network access. Their results must be labeled `local_mock`
or `strict_replay`; neither is official model-quality or native performance
evidence.

## Development live workflow

The development build also supports a direct, warning-only OpenRouter lane for
the first-run wizard and local TUI development. It resolves the key only when
the live command is launched; setup never contacts OpenRouter and never stores
the key. After creating a provider selection and plan, run:

```sh
export OPENROUTER_API_KEY='your-key'
asb run /absolute/path/EXPERIMENT.toml \
  --provider-selection /absolute/path/selection.json --live-provider
```

The selected launch must be an OpenRouter OpenCode or OpenDesk adapter. The
command performs real HTTPS provider calls and fails with an actionable error
if the key is absent; it never silently switches to local/mock. The key is
passed only to the short-lived selected child and is not written to plans,
run manifests, logs, reports, or cassettes. This development lane is not a
production authentication or signature claim.

## Explicit production live workflow

1. Enroll only a logical credential reference through the runtime-owned control
   service. Never put secret bytes in argv, environment exports, plans, logs,
   manifests, cassettes, or reports.
2. Capture a fresh `provider-catalog` response and preserve its exact digest.
3. Create `provider-plan` for one catalog-advertised provider/model and the
   complete agent set. Reject stale, mixed, duplicate, or unsupported selections.
4. Validate with `plan`, then execute `run` or `sweep` with explicit live
   admission. Runtime-owned authority, egress, cancellation and teardown remain
   mandatory; provider reachability is optional supplementary evidence.
5. If recording is authorized, use `record-live` with the required network,
   cost and persistence acknowledgements. Redaction and content addressing occur
   before a cassette is sealed.
6. Use `replay-offline` only with the exact cassette/provider/agent binding.
   Replay authenticates runtime-issued authority, denies provider egress, and
   has no live-provider fallback.
7. Compare live and replay run directories only after preserving their evidence
   labels and manifests; replay does not establish fresh model quality.

The production process entry point does not construct live authority: callers
that provide a runtime/control owner inject the opaque dispatch source through
the authenticated composition boundary. That boundary returns only a
runtime-minted source; policy, credential capabilities, lease and relay roots,
namespace identity, tools, cancellation and teardown remain private to
runtime/control. The direct development lane above is intentionally separate
and warning-only.

See [Provider-aware launches](../PROVIDER_LAUNCH.md) for the exact catalog
snapshot and support matrix, and [Record once, replay offline](record-replay.md)
for cassette boundaries.
