# Offline quickstart

This quickstart exercises the released command boundary without credentials,
downloads, provider traffic, or paid API calls. Use the pinned Rust toolchain
from the repository root:

```sh
cargo build --locked --workspace
cargo run --locked -p asb-cli -- doctor
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

The current CLI does not provide provider recording, replay selection, workload
download, installation, or live-provider setup. Those must not be inferred from
the in-tree replay libraries.

## Tested command support

This table is checked against live `doctor` output and
[the guide contract](examples/guide-contract.json):

| Command | Status |
| --- | --- |
| `doctor` | supported |
| `plan` | supported |
| `run` | supported |
| `sweep` | supported |
| `compare` | supported |
| `report` | supported |
| `serve` | supported |
| `record` | unsupported |
| `replay` | unsupported |
