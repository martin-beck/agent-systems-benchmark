# Measurement methodology

## Questions and evidence boundaries

Measure independent concurrent agent sessions first. Cooperative teams are a
separate scenario with explicit team size, inter-agent communication and scoring.
Never confuse development workers with benchmark clients.

Report live-LLM, recorded-response replay and synthetic-response results separately.
Replay measures agent/runtime/system scalability with controlled model responses;
it does not measure fresh reasoning quality or prove deterministic wall-clock timing.
A container changes user space, not the host kernel. Native distribution/kernel
claims need a booted VM or physical host. Emulation is functional evidence only.

## Metrics

| Group | Required observations |
| --- | --- |
| Responsiveness | Queue delay, launch/readiness latency, request-to-first-byte, first meaningful response, completion p50/p95/p99 |
| Work | Completed and correctly solved tasks per second; failures, timeouts, cancellations, retries |
| Quality | Independent hidden tests, regression preservation, patch validity; compile and lint results by workload |
| CPU | User/system CPU time and utilization, per-session cgroup plus separate host totals |
| Memory | Current/peak cgroup memory, RSS/PSS where available, swap, OOM events |
| Kernel | CPU/memory/I/O PSI, runnable queue, context switches, faults, system CPU, throttling |
| Storage/network | Bytes, operations, latency where available, socket counts and retransmits |
| Agent/model | Tool calls, tokens and usage availability, request count, model wait, optional declared cost |
| Harness | Collector/replay CPU and memory, queue delay, timer lag and dropped samples |

Load average alone is not kernel load. Kernel diagnostics via perf/eBPF are optional
capabilities and require recorded permissions and overhead. Missing measurements
remain unavailable. Mandatory missing metrics make an SLO decision inconclusive.

## Capacity experiment

1. Pin workload mix, input revisions, agent/model settings, seeds, CPU/NUMA topology,
   kernel, power policy, affinity, cgroup limits, storage/cache mode and network mode.
2. Verify one-session correctness and run an unmeasured warmup.
3. Explore concurrency 1, 2, 4, 8, ... up to the configured safety cap, then refine
   around observed transitions. Do not assume a monotone pass/fail curve.
4. Randomize or interleave comparison order; retain cold-start and warm-state
   experiments separately. Repeat boundary points independently.
5. Define sample-count and precision budgets in advance. Report confidence intervals
   for quantiles and success proportions, not just averages. Sparse p99 estimates
   are inconclusive. Use clustered resampling for dependent task runs.
6. Select the highest *tested and repeatedly confirmed* concurrency meeting all
   mandatory bounds. Report the search interval, uncertainty and nearby failures.
7. Stop on OOM, kernel anomalies, runaway resource growth or hard safety limits.
   Preserve partial results and exclude contaminated measurements with reasons.

Support closed-loop fixed-concurrency and open-loop arrival-rate experiments.
Open-loop latency includes waiting from scheduled arrival to completion; count
missed arrivals and timeouts to avoid coordinated omission and survivorship bias.
Successful-only latency must never conceal failed tasks.

Example exploratory policy (user-adjustable, not a universal recommendation):
at least 95% independently correct tasks; p95 completion no more than 1.5 times
the paired one-session baseline; memory below 80% of the experiment allocation;
no OOM or kernel anomaly. Absolute interactive latency limits and PSI bounds
must be declared per workload before collecting results.

Isolate benchmark CPUs, memory and storage activity from CI where possible.
If co-located activity cannot be isolated, mark the run contaminated rather than
publishing it as a dedicated-host capacity result. Include replay service saturation
checks: the response server must have measured headroom beyond the client load.
