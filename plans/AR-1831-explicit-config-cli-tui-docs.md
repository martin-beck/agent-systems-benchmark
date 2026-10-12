# AR-1831 plan: explicit config UX and documentation

1. Update every command help, completion, tutorial, error `Next:` line, and
   JSON schema reference to use setup --output, plan create --config --output,
   and plan/run/sweep PATH --config CONFIG.
2. Ensure human output clearly distinguishes success, warning, failure, and
   migration, names exact paths, announces created directories, and never adds
   progress/status output to JSON mode.
3. Replace legacy TUI handoff assumptions with explicit setup config export and
   config-bound plan execution; retain the boundary that rendering/UI program
   work lives in asb-tui, not ASB.
4. Add help/completion/documentation drift tests that prohibit removed commands
   and provider-selection flags, plus human golden tests for all migration and
   missing-path failures.
