# ASB portable Linux metrics

`asb-metrics` reads process counters from procfs and delegated cgroup-v2 counters without
requiring privilege. It emits the versioned `asb-protocol` metric types. Every metric carries a
stable ID, UCUM-style unit, aggregation, scope, source, and nominal temporal resolution.

The portable collector covers:

- process user/system CPU time, resident memory, minor/major faults, and physical read/write I/O;
- cgroup CPU usage and throttling, current/peak memory, faults, aggregate block I/O, and
  CPU/memory/I/O pressure totals; and
- collection overhead and available/unavailable counts for every collection attempt.

Missing, permission-denied, malformed, and overflowing sources produce explicit unavailable
values. They are never replaced with zero. A cgroup-relative target rejects absolute paths,
parent traversal, and platform-specific path prefixes.

Counter values are cumulative. Process scope is the single requested PID, not its descendants;
cgroup scope includes every process charged to that cgroup. Linux may account buffered file I/O
later or not at all, so the collector reports kernel `read_bytes`/`write_bytes` counters and
does not infer workload byte counts. Pressure totals are cumulative stall time, not percentages.

The collector does not claim attribution for activity outside the selected process or cgroup,
does not provide privileged perf/eBPF data, and does not infer missing kernel capabilities.
Files are read sequentially, so one collection is not an atomic kernel snapshot. The v1 result
contract stores numeric samples as IEEE-754 doubles; integer counters above 2^53 may lose unit
precision. A zero descriptor resolution means that the kernel source does not publish a temporal
resolution, not that the measurement is exact.
