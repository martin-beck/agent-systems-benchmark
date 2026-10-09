# Benchmark quickstart: offline first, live by explicit opt-in

This quickstart exercises the released command boundary without credentials,
downloads, provider traffic, or paid API calls. Use the pinned Rust toolchain
from the repository root:

```sh
cargo build --locked --workspace
cargo run --locked -p asb-cli -- doctor
cargo run --locked -p asb-cli -- capabilities --format json
cargo run --locked -p asb-cli -- project init .
cargo test --locked -p asb-cli --test guide_examples -- --nocapture
```

The final command runs the executable guide fixture through `plan`, `run`,
`sweep`, `compare`, and `report`. It creates a digest-pinned local
`batch-stdio-v1` agent in a private disposable directory, executes the original
bug-fix workload, validates JSON output and durable manifest/journal artifacts,
then removes the directory. It also proves that a stale plan schema and the
missing-authority `replay-offline` command fails closed.

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
asb setup
asb provider-catalog
asb setup --agent AGENT --provider-profile PROVIDER --model MODEL --persist
asb workload-catalog
asb plan create --workload WORKLOAD --agent AGENT \
  --agent-executable /absolute/agent --output /absolute/experiment.toml --use-config
asb plan /absolute/path/experiment.toml
asb run /absolute/path/experiment.toml
asb report /absolute/result/root/runs/RUN_ID
```

`plan` has no launch effect. `run` executes one capacity point. `sweep`
executes the deterministic bounded capacity order. Human-readable output is
the default; pass the global `--json` flag (for example, `asb --json plan ...`)
for the stable machine-readable envelope. Progress is on stderr. A nonzero
exit and structured error are authoritative.
Never treat absent measurements as zero or a failed/inconclusive point as pass.
Catalog commands are user-choice boundaries: substitute only values displayed
as runnable/setup-supported. Subsequent `Next:` lines are copyable, validated
commands and lead from plan validation through run/sweep, report, and compare.

## Complete benchmark workflow

The production-shaped workflow is deliberately explicit:

1. Enroll a logical credential reference through the runtime-owned control
   service; the secret value never enters the CLI arguments or evidence.
2. Run `asb --json provider-catalog`, retain its `catalog_sha256`, and create a
   digest-bound selection with `asb provider-plan`.
3. Validate the experiment with `asb plan`, then run one point with `asb run`
   or a bounded matrix with `asb sweep`.
4. For development and CI, select the credential-free local/mock path. For a
   provider, the development build can make a real OpenRouter call with
   `OPENROUTER_API_KEY` and `--live-provider` after explicit selection. Missing
   credentials remain warning-only during setup, but an explicitly requested
   live run fails clearly rather than silently using a mock. Production live
   execution still requires explicit operator admission and a runtime-issued
   credential capability; provider reachability is optional supplementary
   evidence, never an offline qualification requirement.
   For a selection-driven online benchmark over one or all configured agents,
   use `asb benchmark-live EXPERIMENT.toml --provider-selection selection.json
   --online [--sweep]`. The `--online` flag is mandatory; this route never
   falls back to local/mock or replay. Human-readable output is the default;
   add the global `--json` flag for automation.
5. To preserve a runtime-authorized exchange, use `record-live` with explicit
   capture acknowledgements, then use `replay-offline` against the exact
   content-addressed cassette. Replay denies provider egress and cannot fall
   back to a live provider.

The exact provider/model/agent support boundary is the digest-anchored matrix in
[Provider-aware launches](PROVIDER_LAUNCH.md). A catalog-selectable cell is not
native, official, or quality qualification; retain those evidence labels in every
report.

## Selecting one provider for several agents

Provider selection is noninteractive and fail-closed. First save the advertised
catalog identity, then use that exact identity to export a selection manifest:

```sh
asb --json provider-catalog > catalog.json
# Read CATALOG_SHA256 from catalog.json.
asb --json provider-plan --catalog-sha256 CATALOG_SHA256 \
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

Install the optional standalone application with `asb tui install`, then launch
it with `asb tui`. ASB only authenticates, installs, and delegates lifecycle
operations; all terminal UI code lives in the asb-tui repository. See the
[optional TUI lifecycle](ASB_TUI_LIFECYCLE.md) for trust, offline, XDG, status,
and removal semantics.

Fresh development installs resolve the current `main` head of both the ASB and
asb-tui repositories and default to the `dev` channel. The install/status JSON
projection includes both source identities and a SHA-256 digest of the
content-addressed `active-channel.json` diagnostic manifest under the private
TUI installation root. Missing development authentication, signatures, and key
management are recorded as warnings only; stable lifecycle verification keeps
its separate fail-closed signed-release policy. A stale ASB binary is rejected
with `dev_source_identity_stale` rather than silently installing a different
source pair.

## Recording once and replaying later

Recording is opt-in and requires a bounded `RecordingCapture` JSON envelope
with explicit `record`, `network`, and (when nonzero) `cost` acknowledgements.
The command redacts and atomically seals the capture; its stdout contains only
catalog metadata, never prompts, responses, or credentials:

```sh
asb record-live CAPTURE.json CASSETTE.json --local-mock --confirm-record
asb replay-offline CASSETTE.json PROVIDER_PROFILE_SHA256 AGENT
```

`record-live` requires the explicit confirmation flag and seals a bounded,
redacted capture. In local qualification (`--local-mock`) it does not contact a
provider. Replay authenticates the cassette, requires runtime-issued authority,
an exact provider-profile and agent match, labels the result `strict_replay`,
and denies provider network access.
The checked-in qualification path is local/mock; production provider capture
remains an explicitly supervised runtime integration and is not required for
offline qualification.
The legacy aliases `asb record CAPTURE.json CASSETTE.json` and
`asb replay CASSETTE.json PROVIDER_PROFILE_SHA256 AGENT` remain accepted for
compatibility, but the explicit names above document the complete workflow.
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
| `setup` | supported |
| `capabilities` | supported |
| `project init` | supported |
| `provider-catalog` | supported |
| `adapter-catalog` | supported |
| `workload-catalog` | supported |
| `easy` | supported |
| `tui` | supported |
| `config` | supported |
| `auth` | supported |
| `provider-plan` | supported |
| `plan` | supported |
| `run` | supported |
| `sweep` | supported |
| `benchmark-live` | supported |
| `compare` | supported |
| `report` | supported |
| `record-live` | supported (local mock) |
| `record-campaign` | supported (local mock) |
| `replay-offline` | supported (runtime authority) |
| `record` | supported |
| `replay` | supported |
| `completion` | supported |
| `serve` | supported |
