# ASB terminal settings wizard

`asb-tui` is an independent frontend. Its state model accepts only choices
advertised by a negotiated runner catalog, keeps credential values
unrepresentable, and separates validation and plan creation from launch.

This bounded slice supplies the settings model, plain startup shell, reconnectable
run control, and a bounded recent-run projection. History paging retains only
privacy-reviewed summaries, rejects duplicate or stale pages before mutation, and
requires explicit confirmation for repeat and sensitive-artifact inspection.

The `MultiAgentWizard` model adds a negotiated multi-select flow: a runner
advertises each provider profile together with its complete compatible-agent
set, the user can search and select several agents, and only one profile that
covers every selected agent can be chosen. Selection is canonicalized before
the privacy-safe review and explicit plan confirmation. Stale catalogs,
duplicates, incompatible profiles, unsupported choices, and non-ASCII or
oversized searches fail closed; no launch occurs from selection or review.

The `RecordingWorkflow` model supplies the corresponding record/replay journey.
It exposes only exact compatible cassette choices, keeps near matches with a
typed unavailable reason, and requires explicit acknowledgements for live
recording's network, cost, and persistence consequences. Replay is labeled
`strict_replay` and carries a denied-network policy; no launch is implied by
catalogue display or review.

The current control protocol exposes analysis only as a run count and opaque
artifact digest. The TUI therefore marks that projection as unable to support an
unqualified comparison. Rich history provenance, compatibility/confounder
evidence, interactive rendering, and transport wiring remain follow-up work; this
crate does not claim those surfaces.
