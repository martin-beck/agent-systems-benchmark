# Workload catalogue

The machine-generated [workload catalog matrix](generated/WORKLOAD_CATALOG.md)
is the parity view for registry provenance, local fixture adaptation, platform
evidence, and CLI inventory. Run `python3 tools/quality/generate_workload_catalog.py`
to verify it; CI rejects stale generated output.

The machine-generated [literature parity artifact](generated/literature-parity-v1.json)
reconciles every benchmark and framework named by this document, `RELATED_WORK.md`,
or `PLAN.md` to exactly one registry identity. Run
`python3 tools/quality/reconcile_literature.py` to verify it. Framework and harness
records remain visible for provenance but are never executable workloads without an
independent task protocol and grader.

## Initial portable software engineering suite

Implement these small, original fixtures through extension API v1 before importing
large datasets. Each has deterministic preparation, isolated workspace, hidden
independent grader, pinned toolchain, reference patch and intentionally bad patches.

| Workload | Task and independent oracle |
| --- | --- |
| bug-fix | Diagnose a failing parser edge case; hidden tests pass and existing tests remain green |
| feature-addition | Extend a small CLI from an explicit specification; behavior and compatibility tests |
| refactoring | Change internal structure while preserving tested observable behavior |
| test-generation | Add tests that kill seeded faults; protected grader prevents trivial self-reported success |
| dependency-migration | Update a pinned API dependency from an offline fixture; compile and regression checks |
| build-repair | Repair a broken build/configuration; clean isolated rebuild and executable smoke test |
| repository-navigation | Locate a behavior and explain its code path; machine-checkable file/symbol evidence |

Use Rust, Python, Go and C fixture projects in stages; adapters handle fixture
toolchains independently from the Rust framework. Limit initial dependencies.
Tests/grading artifacts are protected from the agent's write scope. Collect patch
and failure evidence without publishing task secrets or raw private transcripts.

## Established benchmark candidates

| Benchmark | Role in ASB | Priority / caveat |
| --- | --- | --- |
| [SWE-bench Lite / Verified](https://www.swebench.com/SWE-bench/guides/datasets/) | Repository issue repair with independent tests | First external repository suite; preserve official grading |
| [Terminal-Bench](https://www.tbench.ai/benchmarks) | Multi-step terminal, build and debugging work | First terminal suite; pin a published version and use its harness where practical |
| [Aider Polyglot](https://aider.chat/docs/benchmarks.html) | Multi-language editing with test feedback | Early integration; normalize attempts and model budgets |
| Exercism Tracks | Pinned multi-language exercise fixtures | Provenance-only candidate; preserve per-exercise licenses and keep evaluator evidence unavailable |
| [SWE-bench Pro](https://github.com/scaleapi/SWE-bench_Pro-os) | Longer repository changes | Later stress workload; substantially heavier execution |
| [BigCodeBench](https://github.com/bigcode-project/bigcodebench) | Library-oriented code generation | Component workload, not complete agent evaluation |
| [HumanEval+ / MBPP+ via EvalPlus](https://github.com/evalplus/evalplus) | Fast code correctness controls | Small synthetic controls; weak proxy for repository engineering |
| [LiveCodeBench](https://github.com/LiveCodeBench/LiveCodeBench) | Time-windowed coding evaluation | Pin release/window; separate code-generation claims from agent systems claims |
| [SWE-Lancer](https://github.com/openai/frontier-evals/tree/main/project/swelancer) | Long-horizon repository repair and proposal decisions | Archived source boundary; license/archive, task-image, contamination-cutoff, and native evidence remain unqualified |
| [SWE-rebench](https://huggingface.co/datasets/nebius/SWE-rebench) | Continuously refreshed, decontaminated repository repair | Repin every evaluation window; task images, reset, grading, and native evidence remain unqualified |

The AR-0404 catalogue also records four optional, provenance-only suite boundaries in
`crates/asb-workloads/registry/v1/external-workloads.json`: SWE-bench Pro (`main-ca10a60a`),
BigCodeBench (`v0.2.4-9059fb84`), EvalPlus (`v0.3.1-e5d0ed0b`), and LiveCodeBench release v6
(`release-v6-28fef95e`). Each entry pins its source archive, dataset revision, license, evaluator
identity, and explicit-download/non-vendored policy. All evaluator and native-platform cells
remain `planned` or unsupported until immutable evaluator images, SBOMs, reset/fault evidence,
and real native runs are independently qualified. These are code-generation or correctness
controls and must not be reported as complete agent-performance results.

### Code-generation control semantics

The code-generation controls use a separate comparison namespace from
repository-agent workloads. BigCodeBench and EvalPlus (including the
HumanEval+ and MBPP+ split identities) are function-level controls with
`function-correctness-pass-rate` or `expanded-test-pass-rate` metrics. LiveCodeBench
is a time-windowed control with `time-windowed-code-generation-pass-rate`; its
`release_v6` dataset boundary is part of the identity and must not be replaced by
an unpinned latest snapshot. Local development runs exercise only the deterministic
ASB fixture and report the namespace `code-generation.function-level.v1` or
`code-generation.time-windowed.v1`. They do not contact a provider, execute an
upstream evaluator, or contribute scores to repository-agent comparisons.

AR-0406 adds SWE-Lancer and SWE-rebench as explicit-download, non-vendored, provenance-only
boundaries. SWE-Lancer is pinned to frontier-evals commit `51052ced` and remains unavailable for
execution until its archived source/license and offline evaluator boundary are repinned. SWE-rebench
is pinned to SWE-bench-fork `e4907b7a` and dataset revision `89cdfbab`; its CC-BY-4.0 dataset,
continuously changing task window, and missing immutable task-image/evaluator evidence require a
fresh contamination cutoff for each future evaluation. Neither entry claims an evaluator or native
platform result.

Before enabling any external suite, record code and dataset licenses separately,
task revision, evaluator version, image digests and redistributable assets.
Acquire datasets explicitly; do not vendor them into the source repository.
Published x86 images do not imply native arm64 support. Rebuilt arm64 tasks need
oracle parity evidence; otherwise show an unsupported matrix cell.
Retain original benchmark rules and label every adaptation and excluded instance.
Do not combine incompatible scores into an unqualified global ranking.

## Validity registry contract

The machine-checked registry in `crates/asb-workloads/registry/v1` binds validity to
an exact workload content and scorer revision. It records acquisition provenance,
SPDX license, split selection, baseline evidence, exposure or salted holdout-set
identity, dependency pins, disclosed adaptations, platform status, limitations, and
optional host-local performance calibration. Missing evidence stays `planned`; a
container, cross-build, or simulated run never becomes `native-tested`.

Native evidence is bound to platform ID, architecture, booted kernel, public run ID,
date, and artifact digest. Adapted workloads require separately content-addressed
semantic-parity evidence. A speedup threshold is valid only for its opaque host class
and paired uncertainty evidence; thresholds do not transfer silently between hosts.
The checked-in original suite is fully public and has no holdout, contamination-
resistance, native workload, or performance claim.

The external registry also inventories the interactive workload boundaries
AgentBench, tau-bench, and AgentDojo. They are explicit-download, non-vendored
`executable-candidate` records only; task images, reset/scoring parity, licenses,
and platform evidence must be qualified by later adapter ARs. Harbor, Inspect AI,
and HAL are harness/framework boundaries, recorded as `methodology-only` and
never selectable as benchmark tasks. AgentOps, HELM, and AI Agents That Matter
are likewise methodology references and never selectable or executable workloads.

Every executable-candidate family also has a bounded ASB-owned development mock
fixture. The mock uses an in-process deterministic model double, denies egress,
and emits provenance-bound workload, adaptation, evaluator, mock, scorer, and
result digests. It is suitable for offline contract and CLI development only;
it never promotes an official evaluator, dataset, native-platform cell, or
leaderboard result to qualified evidence.

Interactive families have an additional fixture-only adapter. `agentbench`
models resettable stateful environments; `tau-bench` validates bounded tool
calls and simulated-user turns and reports repeated-trial pass@k/pass^k
outcomes; and `agentdojo` keeps useful completion separate from safety-policy
violations. All three bind an exact task revision and local scorer revision,
reject stale revisions, unsafe tools, malformed users, reset leakage, and
scorer drift before producing content-free evidence. `fixture_only` catalog
selection never invokes an upstream evaluator or provider.

## CLI catalog dispatch

The CLI plan, stored-run validation, and execution paths resolve workload IDs
through the unified catalog. Built-in IDs retain their protected grader, while
literature IDs use only the bounded offline local fixture and deterministic
mock scorer. Methodology-only records such as AgentOps, HELM, and AI Agents
That Matter are rejected before plan or workspace effects; they are references,
not executable workloads. Unknown and malformed identities fail closed.

Repository-repair and terminal-system entries expose stable capability tags,
their pinned dataset/task revision, and declared attempt budget in the catalog.
These fields are descriptive provenance: they do not qualify an upstream
evaluator. Local fixture execution remains the only offline development path;
external or native qualification must be supplied separately and matching
evidence is required before a plan can claim an official score.

Long-horizon and performance entries additionally expose metric kind, immutable
evaluation window, contamination-cutoff state, and source archive status. A
refreshed suite without a pinned window/cutoff remains unavailable; paired
performance, uncertainty, hardware-control, and correctness evidence are not
collapsed into repository-repair scores.
