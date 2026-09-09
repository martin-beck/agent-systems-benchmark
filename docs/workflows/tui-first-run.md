# First benchmark with the TUI/control surface

The TUI uses the same concepts as the CLI but keeps state in a negotiated
frontend model. The current `asb-tui` executable is only a startup shell; it
requires a negotiated local frontend connection. There is intentionally no
documented keyboard shortcut until a renderer publishes one.

## Settings and plan

The negotiated renderer should expose this sequence:

1. `WizardCatalog` advertises bounded agents, workloads, providers, and
   recording choices.
2. `Wizard::apply` accepts settings; `Wizard::dry_run` emits
   `ControlCall::ValidateSettings`.
3. Show the validation result and ask for an explicit confirmation.
4. `Wizard::confirm_plan` emits `ControlCall::CreatePlan`; only the runner may
   create the durable plan.

For several agents, use `MultiAgentWizard`: search with `search_agents`, select
   agents with `select_agents` or `toggle_agent`, inspect `compatible_providers`,
   choose one provider with `select_provider`, then call `dry_run` and
   `confirm_plan`. An incompatible or stale catalog must remain unavailable.

The exact model/API names above are the supported contract. A renderer may
choose its own keys, but must show the same names and must not launch from a
selection screen.

## Launch, follow, finish, and compare

After the runner acknowledges the plan, `RunControl` is the lifecycle owner:

1. `RunControl::launch("<explicit-confirmation>")` emits
   `ControlCall::Launch` and enters `LaunchPending`.
2. Accept the identity-matching `RunSummary`; the phase becomes `Following` or
   a terminal state.
3. Poll `status_call` and `events_call`; accept only monotonic, identity-matched
   projections. Render `Planned`, `Prepared`, `Running`, `Collecting`, and the
   terminal states `Completed`, `Failed`, `Cancelled`, or
   `NeedsReconciliation`.
4. Once terminal, use the runner's report/compare commands from the [CLI
   route](cli-first-run.md#follow-finish-and-compare). The output contract is
   shared; the TUI does not reinterpret missing evidence as success.

## Reconnect and recover

On frontend restart, construct `RunControl::reconnect` from the saved plan
reference and the latest runner summary. A mismatched plan, run, attempt, or
revision is rejected as a stale projection. Refresh status/events before
trying again. If the runner reports `NeedsReconciliation`, stop launching and
follow [Troubleshooting](troubleshooting.md#needs-reconciliation).

## Current limitation

Running `asb-tui` alone is not a complete benchmark journey today: it prints
the negotiated-connection requirement and exits. This is a supported fail-closed
boundary, not evidence that a standalone terminal renderer exists.
