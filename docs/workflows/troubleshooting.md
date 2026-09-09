# Troubleshooting and recovery

## Validation fails before launch

Run `asb doctor`, then rerun `asb plan`. Check absolute, disjoint result/work
roots, the executable SHA-256, workload revision, architecture, and (for live
providers) the catalog and selection digests. A failed plan must not create a
result or work root.

## Provider selection is stale or incompatible

Run `asb provider-catalog` again and regenerate `selection.json` with its exact
catalog digest. Never edit a selection manifest by hand. Verify every selected
agent is advertised by the chosen provider; use separate explicit plans for
per-agent provider profiles.

## Run is failed or cancelled

Keep the structured result, manifest, journal, and cleanup outcome. Inspect
`asb report RUN_DIRECTORY`; retry only after deciding whether the failure is a
real workload result, a bounded timeout, or an environmental interruption.
Missing observations are not zero and do not pass a benchmark.

## Needs reconciliation

`needs_reconciliation` means the frontend projection cannot safely infer the
runner's durable state. Stop launching and reconnect with the exact plan/run/
attempt identity. Refresh `status_call` and `events_call` until the runner
returns a monotonic acknowledged summary. Do not delete a durable run to make
the UI look clean.

## Replay is rejected

Check the cassette SHA-256, exact provider-profile digest, selected agent, and
schema/version. Corrupt, incomplete, redaction-ambiguous, or near-match
cassettes must remain rejected. Replay should never contact a provider.

## TUI prints a connection requirement

This is expected for the current standalone `asb-tui` shell. It is not an
interactive renderer. Attach it to a negotiated local frontend connection or
use the CLI route; do not assume undocumented keyboard shortcuts.

## Evidence and support limits

Record the ASB commit, toolchain, workload/scorer revisions, platform, provider
profile identity, and exact command. See [reproducibility](../REPRODUCIBILITY.md),
[quality gates](../QUALITY_GATES.md), and [platform support](../PLATFORMS.md).
