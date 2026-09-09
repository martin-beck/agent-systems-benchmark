# Workload catalogue

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
