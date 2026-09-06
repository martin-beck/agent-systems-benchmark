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
by this crate. The implementation does not interpolate between capacity points or
assume throughput and quality are monotonic with offered load.

The current boundary uses 95 percent intervals and floating-point arithmetic. It
does not claim autocorrelation correction, sequential-testing correction, bootstrap
inference, or a substitute for repeated independent trials.
When a finite two-sided DKW p95 upper bound is not identifiable at the configured
confidence, the upper bound is absent and a maximum-latency SLO cannot pass.
