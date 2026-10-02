# Operator quickstart

This is the shortest supported path from an installed development build to a
benchmark result. It is intentionally credential-free: the local fixture route
does not contact a provider, download a workload, or incur cost.

## 1. Install and check the boundary

Install the exact release or development bundle into an owner-only directory,
then check the executable before opening the wizard:

```sh
asb doctor
asb setup
```

Operator output is human-readable by default. Add `--json` when a script or
frontend consumes the versioned envelope (`asb doctor --json`,
`asb setup --json`). The compatibility spelling `--format json` is accepted.

If this is a development bundle, setup may print `development-only` warnings
for missing authentication, signatures, or key management. These warnings are
visible and do not block the local fixture journey. They are not evidence of
provider authorization; the production release path remains fail-closed.

## 2. Select settings in the wizard

Use the catalog-driven setup rather than typing an arbitrary provider or model:

```sh
asb setup --agent codex --agent opendesk \
  --provider-profile openrouter \
  --model cohere/north-mini-code:free \
  --credential-env OPENROUTER_API_KEY \
  --persist
```

The wizard presents only catalog-advertised agents, compatible providers, and
models. Select every agent first, then one provider/model profile that covers
the complete selection. The saved configuration contains identities and a
credential-reference digest, never a secret value. Re-run setup to edit the
selection; use `--no-default` when it must not replace shared defaults.

## 3. Run one benchmark

Create or choose a bounded experiment, validate it, then run the local fixture:

```sh
asb plan /absolute/path/EXPERIMENT.toml --use-config
asb easy run /absolute/path/EXPERIMENT.toml --use-config --local-mock
asb report /absolute/path/RESULT_ROOT/runs/RUN_ID
```

`plan` has no launch effect. A local-mock result is labeled development-only
and is not live-provider, model-quality, or native-performance evidence.

## 4. Record once, replay offline

Recording is an explicit opt-in because live capture can expose provider
traffic and cost. For a credential-free walk-through, use the bounded fixture:

```sh
asb easy record /absolute/path/CAPTURE.json \
  /absolute/path/CASSETTE.json --local-mock
asb easy replay-offline /absolute/path/CASSETTE.json \
  PROVIDER_PROFILE_SHA256 codex --local-mock
```

Replay requires an exact cassette, provider profile, and agent match. It denies
provider network access and never falls back to a live provider. A replay is
strict offline evidence, not fresh model-quality evidence.

## 5. Compare terminal runs

Report both terminal run directories, then compare them:

```sh
asb report /absolute/path/live-run
asb report /absolute/path/replay-run
asb compare /absolute/path/live-run /absolute/path/replay-run
```

Only a result with `comparable: true` supports a difference claim. Preserve
the unavailable reason when runs differ in workload, configuration, provider,
or evidence source; never turn missing measurements into zero.

For the paired selection-driven terminal route, see the [asb-tui operator
quickstart](https://github.com/martin-beck/asb-tui/blob/main/docs/OPERATOR_QUICKSTART.md).
