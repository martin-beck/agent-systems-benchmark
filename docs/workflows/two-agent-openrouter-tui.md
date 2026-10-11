# Two-agent OpenRouter and TUI walkthrough

This page is the linked, beginner-oriented route from installation to a
bounded result. It uses the catalog and a credential reference; it never asks
you to paste an API key into a file, command line, screenshot, or report.

## Install and check the boundary

Install a release bundle, or build the development checkout with the pinned
toolchain described in [the platform matrix](../PLATFORMS.md):

```sh
cargo build --locked --workspace
cargo run --locked -p asb-cli -- doctor
cargo run --locked -p asb-cli -- capabilities --format json
```

`doctor` reports missing dependencies without installing them. Fix the named
package with the distribution command in the matrix, then rerun the check.
Keep optional sandbox utilities separate from the required Rust/build tools.

## Select two catalog-advertised agents

Start with a fresh catalog and retain only its digest. The following example
uses the free OpenRouter model advertised by the catalog at the time of
selection; do not substitute a model copied from a web page:

```sh
asb provider-catalog > catalog.json
# Read catalog_sha256 from catalog.json; do not copy the catalog into evidence.
asb provider-plan --catalog-sha256 CATALOG_SHA256 \
  --provider-profile openrouter --model cohere/north-mini-code:free \
  --credential-reference-sha256 CREDENTIAL_REFERENCE_SHA256 \
  --agent codex --agent opendesk > selection.json
asb setup --agent codex --agent opendesk \
  --provider-profile openrouter --model cohere/north-mini-code:free \
  --credential-env OPENROUTER_API_KEY --persist
```

`CREDENTIAL_REFERENCE_SHA256` is a digest of a logical secret reference, not
the secret. Setup does not contact OpenRouter. The provider plan fails closed
if either agent or the model is absent/incompatible in the pinned catalog.

## Run, report, and compare

Use a bounded `EXPERIMENT.toml` (small attempt and concurrency limits) and
validate it before launching. The safe qualification path is local mock:

```sh
asb plan /absolute/path/EXPERIMENT.toml --use-config
asb easy run /absolute/path/EXPERIMENT.toml --use-config --local-mock
asb report /absolute/path/RESULT_ROOT/runs/RUN_ID
asb compare /absolute/path/RUN_A /absolute/path/RUN_B
```

For an explicitly supervised provider run, add `--live-provider` to the
selection-driven command supported by your build. Live traffic and cost are
opt-in; a missing credential is an error and never silently falls back to a
mock. A local-mock result is development evidence, not provider-quality or
native-performance evidence.

To preserve a run for an offline repeat, record with the explicit confirmation
and then replay the exact cassette:

```sh
asb record-live CAPTURE.json CASSETTE.json --local-mock --confirm-record
asb replay-offline CASSETTE.json PROVIDER_PROFILE_SHA256 codex --local-mock
```

Replay denies provider egress. Compare only results marked `comparable: true`.

## Equivalent TUI journey

The standalone `asb-tui` binary is currently a negotiated frontend shell; it
prints a connection requirement rather than pretending to render a complete
benchmark. Install it through `asb tui install`, then follow the paired
[asb-tui operator quickstart](https://github.com/martin-beck/asb-tui/blob/main/docs/OPERATOR_QUICKSTART.md).
The model steps are:

1. Open the catalog and select `codex` and `opendesk`.
2. Select the `openrouter` profile and the catalog-advertised model; show the
   credential reference digest, never its value.
3. Dry-run settings, confirm the plan, and launch only after the runner
   acknowledges it.
4. Monitor `Planned`, `Prepared`, `Running`, and `Collecting`, then accept a
   terminal `Completed`, `Failed`, `Cancelled`, or `NeedsReconciliation`
   projection.
5. Open the report and compare two identity-matching runs.
6. On reconnect, refresh status/events and reconcile a stale or interrupted
   run before launching again. See [TUI first run](tui-first-run.md) and
   [troubleshooting](troubleshooting.md#needs-reconciliation) for details.

This page is also usable without a renderer: the numbered sequence is the
text-only equivalent of the TUI journey. No screenshot is required to infer a
keyboard map or claim that the current shell is a full TUI.

## Offline alternatives and evidence labels

Use `--local-mock` for credential-free development and `replay-offline` for an
exact sealed cassette. Use live OpenRouter only when the operator has reviewed
the provider, cost, network, and credential consequences. Label reports as
development, live, or strict replay; do not promote any of these labels to
native platform qualification.

The syntax-only contract for the offline setup/readiness portion is
[`two-agent-openrouter-v1.json`](../../tools/tutorials/two-agent-openrouter-v1.json).
Validate it and all other tutorial contracts with:

```sh
python3 tools/tutorials/check_freshness.py
```
