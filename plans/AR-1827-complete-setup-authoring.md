# AR-1827 plan: complete explicit setup authoring

1. Require `asb setup --output CONFIG`; make missing output, parent creation,
   existing target, overwrite, unsafe path, and failed atomic write distinct,
   concise human and JSON outcomes.
2. Build the interactive flow and equivalent noninteractive flags for agent
   selection/readiness, provider selection, credential-reference mechanism,
   remote model enumeration, agent/provider/model compatibility, workloads,
   monitor/measure groups and individual measures, and single-run/sweep bounds.
3. Keep unsupported choices visible with actionable reasons; do not silently
   substitute a model, provider, local mock, replay, or measure.
4. Record an internal, digest-addressed registry snapshot in the explicit
   config so setup is self-contained from the user's perspective but refresh
   and staleness can later be diagnosed.
5. Require explicit live/network/cost acknowledgement for live mode, while
   allowing development setup and offline planning to proceed without a live
   credential.
6. Add interactive-PTY and noninteractive fixtures spanning every supported
   provider/agent/model/workload/measure choice, including redaction and
   human-output assertions.
