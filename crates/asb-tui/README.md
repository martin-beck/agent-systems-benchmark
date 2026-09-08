# ASB terminal settings wizard

`asb-tui` is an independent frontend. Its state model accepts only choices
advertised by a negotiated runner catalog, keeps credential values
unrepresentable, and separates validation and plan creation from launch.

This bounded slice supplies the settings model and plain startup shell.
Interactive rendering, transport wiring, and run control remain follow-up work;
this crate does not claim those surfaces.

The `MultiAgentWizard` model adds a negotiated multi-select flow: a runner
advertises each provider profile together with its complete compatible-agent
set, the user can search and select several agents, and only one profile that
covers every selected agent can be chosen. Selection is canonicalized before
the privacy-safe review and explicit plan confirmation. Stale catalogs,
duplicates, incompatible profiles, unsupported choices, and non-ASCII or
oversized searches fail closed; no launch occurs from selection or review.
