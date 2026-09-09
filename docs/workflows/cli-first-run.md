# First benchmark with the CLI

This route is credential-free and uses the repository's public
`original.bug-fix` workload. Replace `EXPERIMENT.toml` with an absolute path to
your validated plan and `RESULT_ROOT/runs/RUN_ID` with the paths emitted by the
plan.

## 1. Prepare and validate

```sh
cargo build --locked --workspace
cargo run --locked -p asb-cli -- doctor
asb doctor
asb plan /absolute/path/EXPERIMENT.toml
```

Expected state: `doctor` reports the supported command/workload inventory and
`plan` returns structured JSON with command `plan`. No result or work directory
is created by `plan` when validation fails.

For a live provider, first bind a catalog and a logical credential reference;
the reference digest is not a credential value:

```sh
asb provider-catalog > catalog.json
asb provider-plan --catalog-sha256 CATALOG_SHA256 \
  --provider-profile openai --agent codex --agent opendesk \
  --credential-reference-sha256 CREDENTIAL_REFERENCE_SHA256 > selection.json
asb plan /absolute/path/EXPERIMENT.toml --provider-selection selection.json
```

## 2. Launch one point or a bounded sweep

```sh
asb run /absolute/path/EXPERIMENT.toml --provider-selection selection.json
asb sweep /absolute/path/EXPERIMENT.toml --provider-selection selection.json
```

The command prints JSON on stdout and progress on stderr. A successful response
contains a run identifier and terminal evidence. A nonzero exit with a
structured error is authoritative; never turn a missing measurement into zero.

## Follow, finish, and compare

Use the local control endpoint when the plan is served by a runner:

```sh
asb serve /absolute/path/CONTROL.toml
```

For a completed run, inspect its evidence and then compare two compatible run
directories:

```sh
asb report /absolute/RESULT_ROOT/runs/RUN_ID
asb compare /absolute/RESULT_ROOT/runs/RUN_A \
  /absolute/RESULT_ROOT/runs/RUN_B
```

Expected terminal states are `completed`, `failed`, `cancelled`, or
`needs_reconciliation`; `report` validates the journal and manifest rather
than inferring success. `compare` must say `comparable: true` before its
differences are meaningful.

## What can cost money or require access?

The offline route needs no credentials, network, or paid API. A live provider
requires a preconfigured provider profile and a valid logical credential
reference. Do not use `run` or `sweep` until the provider selection is freshly
validated against the catalog and experiment.
## Rootless one-line installation

The supported bootstrap is an HTTPS download into a private temporary directory:

```sh
ASB_MANIFEST_URL=https://downloads.example.invalid/asb/releases/v1/manifest.json \
ASB_MANIFEST_SHA256=PINNED_MANIFEST_SHA256 \
ASB_ALLOWED_SIGNERS_FILE=/path/to/pinned/allowed-signers \
ASB_SIGNATURE_PRINCIPAL=release@asb \
  sh -c 'curl -fsSL --proto "=https" --tlsv1.2 \
    https://downloads.example.invalid/asb/install/bootstrap.sh | sh'
```

The bootstrap requires an explicit SHA-256 for the release manifest; HTTPS alone
is not treated as cryptographic release authentication. It selects one exact
native platform artifact, verifies its size and SHA-256, rejects unsafe archive
topology, verifies its detached signature against the explicit SSH allowed-signers
trust root, installs into an owner-only XDG data directory transactionally, and
refuses to overwrite an existing release or configuration. It never requests
`sudo`, edits shell startup files, stores credential values, or sends prompts to
argv, logs, or artifacts.

For stronger supply-chain assurance, download the script and manifest through a
pinned/offline channel, verify both digests independently, then run the script
with `ASB_SERVICE_MODE=disabled` and start the supervised `asb serve` command
only after reviewing the installed `asb doctor` result. On systems without a
user systemd manager, the installer prints that supervised command and does not
daemonize it implicitly. Set `ASB_NO_TUI=1` for noninteractive installation.

## Lifecycle operations

Run `tools/install/lifecycle.sh status` to inspect the active release, `backup`
before maintenance, `repair` to restore service/configuration drift, and
`rollback` only when the recorded prior release passes `asb doctor`. Upgrades
must first be installed as a new authenticated release by `bootstrap.sh`; no
lifecycle command silently cancels active benchmark runs or mutates durable
results. `uninstall` removes only the user service link and active link, prints
retained paths, and never purges releases, recordings, results, indexes, or trust
state implicitly.
