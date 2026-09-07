# ASB statistical analysis

`asb-analysis` provides dependency-free, deterministic summaries and conservative
SLO decisions. Successful latency samples use R/NumPy type-7 empirical quantiles
with a two-sided Dvoretzky-Kiefer-Wolfowitz band. Quality uses the Wilson score
interval and keeps failures, timeouts, and cancellations in the denominator.
Throughput uses a two-sided Student t interval across independent equal-duration
windows; one window is deliberately inconclusive and unequal windows are rejected.
For more than 31 windows, the 30-degree-of-freedom critical value is retained as
a conservative bound instead of substituting a narrower asymptotic interval.

The checked-in reference vectors were independently calculated from the published
type-7, Wilson, DKW, and Student t formulas. They are fixtures, not values generated
by this crate. Both a finite DKW p95 upper bound and a sample too small to identify
one are covered. Reproduce them with the exact packages in
`tests/reference-requirements.txt` by running `tests/validate_reference_vectors.py`.
The implementation does not interpolate between capacity points or assume
throughput and quality are monotonic with offered load.

The current boundary uses 95 percent intervals and floating-point arithmetic. It
does not claim autocorrelation correction, sequential-testing correction, bootstrap
inference, or a substitute for repeated independent trials.

`analyze_reliability` reports first-attempt pass, empirical pass@k, and
empirical all-k pass^k from complete repeated trials. Failed, timed-out,
cancelled, and planned-but-unstarted attempts stay in exact denominators.
Analysis compares every observed class, epoch, trial, attempt index, and seed
against a bounded pre-execution roster, so omitting an entire hard stratum
fails closed. Reports retain the queue-delay starvation threshold alongside
each aggregate, class, and epoch fairness result.
Mixed-load reports expose every workload/language/difficulty class and execution
epoch in stable order with queue-delay, SLO, and starvation evidence, so an
aggregate cannot silently hide a starved, consistently hard, or degrading
class. These empirical rates do not establish independence, stationarity, or a
population pass probability.

The terminology is grounded in two pinned public references, with no copied
code or runtime dependency: tau-bench commit
`59a200c6d575d595120f1cb70fea53cef0632f6b` (MIT; paper arXiv:2406.12045)
defines pass^k as all k repeated trials succeeding, while Inspect AI tag
`0.3.258` at commit `e72c73f8a514c53ddf55da180e4bedaf8f0362b4` (MIT) models epochs as
repeated executions of each sample. ASB additionally reports the distinct
at-least-one-success pass@k quantity and never substitutes an independence
formula for either empirical rate. These references were inspected on
2026-09-07; validation used Rust and Cargo 1.93.0.
When a finite two-sided DKW p95 upper bound is not identifiable at the configured
confidence, the upper bound is absent and a maximum-latency SLO cannot pass.

Experiment comparison validates both content-addressed manifests before examining
them. It reports a deterministic list of mismatched required dimensions and
permits an unqualified comparison only when that list is empty. Reports expose
field identities rather than raw settings or provenance values. A comparable
classification establishes matched recorded configuration, not causal equivalence:
unrecorded hardware, service, environmental, or temporal effects can still
confound a result and require experimental controls.
