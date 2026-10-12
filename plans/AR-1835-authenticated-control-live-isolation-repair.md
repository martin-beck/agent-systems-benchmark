# AR-1835 plan: authenticated control and live-isolation coverage repair

1. Read the ASB development, architecture, quality and coordination contracts and the AR-1783 evidence trail.
2. Add only test-support code and fixtures needed to drive the existing authenticated ControlServer/ControlClient seam with a deterministic credential-free backend; reject malformed, stale, mismatched and replayed requests before effects.
3. Extend the existing delegated-isolation fixture or add a bounded containerized runner to exercise live-service, launch-factory and sandbox branches without real providers or fabricated authority.
4. Run focused tests, serialized workspace coverage, formatting, clippy, privacy and generated-artifact cleanup through handoffctl; retain only content-minimized durable evidence.
5. Obtain independent exact-head review and publish only after unchanged gates pass. AR-1783 remains independently in progress.

This repair does not alter provider policy, runtime authority construction, coverage thresholds or release gates.
