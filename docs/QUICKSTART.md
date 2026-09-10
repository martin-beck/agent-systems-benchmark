# Offline quickstart

This quickstart exercises the released command boundary without credentials,
downloads, provider traffic, or paid API calls. Use the pinned Rust toolchain
from the repository root:

```sh
cargo build --locked --workspace
cargo run --locked -p asb-cli -- doctor
cargo run --locked -p asb-cli -- capabilities --format json
cargo test --locked -p asb-cli --test guide_examples -- --nocapture
```

The final command runs the executable guide fixture through `plan`, `run`,
`sweep`, `compare`, and `report`. It creates a digest-pinned local
`batch-stdio-v1` agent in a private disposable directory, executes the original
bug-fix workload, validates JSON output and durable manifest/journal artifacts,
then removes the directory. It also proves that a stale plan schema and the
unimplemented `record` and `replay` commands fail.

## Preparing a real plan

A plan is a bounded TOML file with:

- absolute, disjoint result and work roots whose nearest existing directory is
  canonical and not a symlink;
- an absolute regular executable, its lowercase SHA-256, and no unpinned
  arguments;
- one ID from the tested workload list in
  [the guide contract](examples/guide-contract.json);
- a validated experiment manifest whose agent, workload, architecture, and
  content address match the execution;
- a bounded capacity point declaring attempts, concurrency, queue, timeout,
  failure budget, seed, and optional sweep maximum.

Validate before launching:

```sh
asb plan /absolute/path/experiment.toml
asb run /absolute/path/experiment.toml
asb report /absolute/result/root/runs/RUN_ID
```

`plan` has no launch effect. `run` executes one capacity point. `sweep`
executes the deterministic bounded capacity order. Results are JSON on stdout;
progress is on stderr. A nonzero exit and structured error are authoritative.
Never treat absent measurements as zero or a failed/inconclusive point as pass.

## Selecting one provider for several agents

Provider selection is noninteractive and fail-closed. First save the advertised
catalog identity, then use that exact identity to export a selection manifest:

```sh
asb provider-catalog
asb provider-plan --catalog-sha256 CATALOG_SHA256 \
  --provider-profile openai --agent codex --agent opendesk \
  --credential-reference-sha256 CREDENTIAL_REFERENCE_SHA256 > selection.json
asb plan /absolute/path/experiment.toml --provider-selection selection.json
asb run /absolute/path/experiment.toml --provider-selection selection.json
```

`CREDENTIAL_REFERENCE_SHA256` is the digest of the logical secret reference,
not a credential value. The imported selection must remain a bounded regular
JSON file and must match the plan's agent, provider, model, and
`additional_settings_sha256` profile identity. `plan`, `run`, and `sweep`
reject stale catalogs, altered selections, unsupported agents, or mismatched
experiments before creating result/work roots or launching an agent. OpenAI is
selectable. Ollama remains advertised but unavailable until local-daemon
evidence has been verified; there is no silent fallback. Bash completion is
available with `source <(asb completion bash)`.

An independently installed frontend must first run the exact side-effect-free
probe `asb capabilities --format json`. The closed v1 response advertises only
operations backed by the authoritative control API. Unknown fields, versions,
types, formats, or arguments are rejected; the response contains no local paths,
identity, environment, or provider configuration.

## Recording once and replaying later

Recording is opt-in and requires a bounded `RecordingCapture` JSON envelope
with explicit `record`, `network`, and (when nonzero) `cost` acknowledgements.
The command redacts and atomically seals the capture; its stdout contains only
catalog metadata, never prompts, responses, or credentials:

```sh
asb record CAPTURE.json CASSETTE.json
asb replay CASSETTE.json PROVIDER_PROFILE_SHA256 AGENT
```

Replay authenticates the cassette, requires an exact provider-profile and agent
match, labels the result `strict_replay`, and denies provider network access.
Incomplete, corrupt, stale, or incompatible cassettes fail before execution.
The TUI exposes the same choices through `RecordingWorkflow`: compatible
recordings are selectable, near matches carry an explicit unavailable reason,
and live recording shows network and cost consequences before confirmation.

## Tested command support

This table is checked against live `doctor` output and
[the guide contract](examples/guide-contract.json):

| Command | Status |
| --- | --- |
| `doctor` | supported |
| `capabilities` | supported |
| `provider-catalog` | supported |
| `provider-plan` | supported |
| `plan` | supported |
| `run` | supported |
| `sweep` | supported |
| `compare` | supported |
| `report` | supported |
| `record` | supported |
| `replay` | supported |
| `completion` | supported |
| `serve` | supported |
