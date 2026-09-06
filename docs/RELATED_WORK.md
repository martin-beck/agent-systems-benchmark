# Related agent benchmark systems

Research snapshot: 2026-09-06. The listed features come from project
documentation or papers; they are not claims that ASB has implemented them.

| System | Useful model | ASB consequence |
| --- | --- | --- |
| [Harbor](https://www.harborframework.com/docs/core-concepts) | Tasks, agents, environments, trials and jobs; parallel trials; verifier rewards | Preserve distinct setup, agent execution, verification and infrastructure timing. Support importing task definitions where license and semantics allow |
| [Inspect AI](https://inspect.aisi.org.uk/scoring.html) | Multiple scorers, epoch aggregation, deferred scoring, rescoring and structured logs | Keep raw observations and versioned scoring separate so a run can be rescored without rerunning an agent |
| [HAL](https://github.com/princeton-pli/hal-harness) and [paper](https://arxiv.org/abs/2510.11977) | Agent/benchmark independent CLI, local/container/cloud execution, concurrency, cost and trajectories | Model agent scaffold, model/provider and benchmark as separate identities; plan distributed workers only after single-host correctness |
| [AI Agents That Matter](https://arxiv.org/abs/2407.01502) | Highlights cost omission, weak holdouts and reproducibility failures | Enforce experiment comparability, exposure records and quality/cost/resource Pareto reports |
| [Aider Polyglot](https://aider.chat/docs/leaderboards/) | Multilingual exercises; correctness, editing, tokens, cost, timeouts and time | Preserve language strata, protocol/edit failures and repair budgets |
| [AgentBench](https://arxiv.org/abs/2308.03688) | Interactive OS, database and other environments beyond code patches | Keep workload protocol general enough for stateful environments and non-patch graders |
| [tau-bench](https://github.com/sierra-research/tau-bench) | Repeated-trial pass^k reliability and simulated users | Separate reliability from pass@k and version/metre simulated users as independent components |
| [AgentDojo](https://github.com/ethz-spylab/agentdojo) | Utility under tool-use attacks and defenses | Future robustness suites need separate useful-completion and policy-violation scores |
| [AgentOps](https://github.com/AgentOps-AI/agentops) | Execution graphs, integrations and cost observability | Use causal trajectory concepts; observability replay is not evidence of byte/semantic LLM response replay |
| [HELM](https://github.com/stanford-crfm/helm) | Standardized scenarios, adapters, metrics and transparent artifacts | Use methodological lessons; upstream announces maintenance mode beginning 2026-06-01, so it is not selected as a new core dependency |

ASB's intended contribution is a Linux systems benchmark: find sustainable
agent concurrency under declared correctness, latency and resource SLOs while
reporting kernel behavior, platform identity, replay mode and measurement
uncertainty. It should interoperate with established tasks rather than create a
new opaque aggregate leaderboard.

AR-1001 through AR-1007 turn the comparison into implementation work:
comparability, verifier integrity, budgets, reliability/fairness, trace export,
distributed workers and workload validity. AR-0405/0406 add later performance,
reproducibility and evolving workloads.

Further candidates include [SWE-Lancer](https://openai.com/index/swe-lancer/),
[SWE-Perf](https://github.com/SWE-Perf/SWE-Perf),
[SWE-fficiency](https://github.com/swefficiency/swefficiency),
[CORE-Bench](https://github.com/siegelz/core-bench) and
[SWE-rebench](https://swe-rebench.com/about).
The study [Are Performance-Optimization Benchmarks Reliably Measuring Coding
Agents?](https://arxiv.org/abs/2607.01211) reports cross-machine instability,
supporting per-host reference validation and uncertainty-aware speedup scoring.
SWE-bench's current arm64 support is experimental; framework portability never
upgrades an imported workload's evidence status.
