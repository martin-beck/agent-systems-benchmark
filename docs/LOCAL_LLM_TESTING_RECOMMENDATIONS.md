# Deterministic LLM doubles and local inference

Research snapshot: 2026-09-09. This is a source/documentation assessment, not ASB
compatibility certification. Claims require a shared hostile conformance run at the
exact pinned revision before ASB calls them supported.

Primary project sources:

- [MockAgents](https://github.com/mockagents/mockagents/tree/6ddb03e54a14484e5929a19673f0cfd8a1975f07)
- [CopilotKit aimock](https://github.com/CopilotKit/aimock/tree/7323e819bce1b971dc2b7907407401b7d65bbe8c)
- [larsakerlund/llmock](https://github.com/larsakerlund/llmock/tree/9121df067b0a0af1c27f57d3d81f3f128528b586)
- [piyook/llm-mock](https://github.com/piyook/llm-mock/tree/96895387780934fcfbb59311ff99c987dde7f4a0)
- [Ollama OpenAI compatibility](https://github.com/ollama/ollama/tree/13f2fb8c99278469b954429d5541019f4d83a4d0)
- [llama.cpp server](https://github.com/ggml-org/llama.cpp/blob/5266f24da75dc449bd56cbed7addb9c8e4a6a73e/tools/server/README.md)
- [vLLM OpenAI-compatible server](https://github.com/vllm-project/vllm/tree/2cf0a6915ce544dc493a0990f2ea38d81601128a)
- [LocalAI overview](https://github.com/mudler/LocalAI/tree/f7ad3f70eb5d8a0ddf80e08557f0d7df28cf032e)

Release tags and commits below are the assessed identities. Implementing tasks must
refresh upstream facts, then preserve immutable source and artifact digests in their
own evidence rather than relying on a moving documentation URL.

## Recommendation and evidence boundary

Preserve three classes. A deterministic protocol double returns generated public
fixtures and tests adapters, streams, tools and failures, not a model. Strict
cassette replay runs the real agent/tools against content-addressed responses with
provider networking denied; existing `asb-replay` remains authoritative. Local
inference runs a real pinned model/server and is variable live evidence, not a mock
or recorded replay.

Do not adopt a mock server from its README. Run an ASB-owned OpenAI+Anthropic spike
over exact candidates and select at most one credential-free CI double by measured
coverage, determinism, isolation, maintenance and supply-chain cost. Retain pinned
Ollama as local baseline; qualify llama.cpp, vLLM and LocalAI in one profile
workstream. Add adapters only where tested protocols cannot fit the existing
provider/profile boundary.

This extends AR-0312/0313/0315 (Ollama/provider parity), AR-0501..0505 (cassette
contract and fail-closed replay), and AR-0869..0872 (CLI/TUI selection and workflow
docs). AR-0873/0874 retain CI-screenshot and drift-refresh ownership. Mock mode must
never satisfy live requests; replay never falls back; local inference passes identity
probes. Reports keep `synthetic`, `strict_replay`, `local_live` and `remote_live`
separate and never pool them as one benchmark population.

## Deterministic-double candidates

- **MockAgents:** `mockagents/mockagents` v0.5.0 peels to
  `6ddb03e54a14484e5929a19673f0cfd8a1975f07` (Apache-2.0). Docs advertise Go,
  OpenAI/Anthropic, Responses, SSE, tools, scenarios, faults, contracts and replay.
  First spike candidate; independently test every claimed wire behavior.
- **CopilotKit aimock:** v1.40.0 is
  `7323e819bce1b971dc2b7907407401b7d65bbe8c` (MIT). Formerly
  `@copilotkit/llmock`, it advertises programmable fixtures and broad protocols.
  Its warning to set base URL before client construction becomes a hostile case:
  late/bad setup must contact no real provider.
- **larsakerlund/llmock:** v0.1.2 is
  `9121df067b0a0af1c27f57d3d81f3f128528b586` (MIT). It advertises three provider
  shapes, SSE, faults, determinism and replay. Rust 1.96 exceeds ASB's 1.93 pin, so
  test a pinned external artifact without changing the workspace toolchain.
- **piyook/llm-mock:** 3.3.6 is
  `96895387780934fcfbb59311ff99c987dde7f4a0` (MIT). This Node 20 OpenAI-style
  baseline includes random responses/delay and non-loopback examples, conflicting
  with ASB requirements until disabled and proven. Do not confuse the two llmocks.

The common black-box suite covers OpenAI Chat/Responses and Anthropic Messages;
buffered/SSE success; multiline events, usage, tools and finish reasons; parallel
sessions, cancellation/backpressure; errors, disconnect, truncation, invalid JSON
and delay; deterministic repeat/mismatch; zero DNS/outbound access; redacted bounded
logs; clean teardown; and exact provenance. Record pass, fail, unsupported and
untested separately.

## Literature boundary

[AgentRR](https://arxiv.org/abs/2505.17716) reuses recorded traces and summarized
experience, not exact HTTP replay, and proves no systems-performance determinism.
[Deterministic Replay for AI Agent Systems](https://arxiv.org/abs/2607.16200)
describes agrepl capture/matching/isolation; ASB must reproduce stream/isolation
cases and account for uncaptured clock, filesystem, subprocess and scheduler inputs.
[Automated structural testing](https://arxiv.org/abs/2601.18827) supports a fast
mock layer without replacing end-to-end qualification. The
[over-mocking study](https://arxiv.org/abs/2602.00409) motivates the rule that mock
success cannot replace native, replay or live-provider gates.

Existing [replay research](REPLAY_RESEARCH.md) and
[evaluation](REPLAY_EVALUATION.md) remain authoritative for cassettes. This assesses
protocol doubles, not another replay engine.

## Local-inference candidates

- **Ollama:** ASB pins loopback-only 0.33.1 at source commit `13f2fb8c99278469b954429d5541019f4d83a4d0` and a model digest. Its docs describe
  partial OpenAI compatibility and non-stateful Responses. Update server, model,
  templates, routes and wire expectations together.
- **llama.cpp:** v0.4.0 peels to
  `5266f24da75dc449bd56cbed7addb9c8e4a6a73e` (MIT). Its server documents
  OpenAI/Anthropic routes, tool templates, parallel decoding and batching. It is the
  lightweight second-engine candidate; qualify each route/template/parser.
- **vLLM:** v0.28.0 is `2cf0a6915ce544dc493a0990f2ea38d81601128a`
  (Apache-2.0), the GPU/throughput candidate. Pin image, model/tokenizer, parser,
  parallelism and sampling. Model `generation_config.json` may override defaults
  unless `--generation-config vllm` is used; API keys alone are not isolation.
- **LocalAI:** v4.9.0 is `f7ad3f70eb5d8a0ddf80e08557f0d7df28cf032e`
  (MIT). It fronts several engines. Record frontend and backend artifacts,
  model/tokenizer and configuration; do not call it one inference engine.

A local profile records immutable frontend/backend artifacts, model/tokenizer
digests, quantization/template, dialect/routes/parser, sampling, CPU/GPU/runtime,
thread/batch/context/scheduler settings, isolation, probe, warm-up, concurrency and
teardown. Model bytes need not repeat; trials measure variability.

The grouped profile workstream maintains a fail-closed machine-readable evidence
boundary in [`tools/local-inference-profiles/profiles-v1.json`](../tools/local-inference-profiles/profiles-v1.json).
Profiles without complete immutable model, tokenizer, runtime, hardware, and
repeated-trial evidence remain explicitly `unqualified` and non-selectable; they
must not be silently routed through Ollama or a mock.

## Independent grading

Where a benchmark claims independent grading, the candidate execution path cannot
grade itself. The grader must use a separately selected and proven provider/model
profile, credentials, process state and network authorization. A synthetic double
may test grader protocol plumbing with public fixtures, but its score is synthetic
test evidence; a candidate local model, the same engine/model profile, or replayed
candidate output cannot be promoted to independent model-quality evidence.

AR-0892 owns the acceptance boundary: retain grader provider/model/artifact and
evidence-class provenance, reject candidate/grader identity overlap and unavailable
grader fallback, and publish a hostile same-profile rejection plus an independently
provisioned grading artifact for every quality comparison claiming independence.

## Privacy, provenance and delivery

Use generated public prompts, tools and replies. Reject credentials, private hosts,
developer paths, raw user data and unbounded bodies. Keep the existing redacted,
bounded cassette format. Resolve tags to commits, verify licenses/digests, run
unprivileged with bounded resources, and expose no public listener.

| AR | Deliverable | Depends on |
| --- | --- | --- |
| AR-0879 | Research decision/task graph | Existing provider/replay/workflow work |
| AR-0888 | OpenAI+Anthropic mock conformance | AR-0879 |
| AR-0889 | Fixture/privacy-safe evidence contract | AR-0888 |
| AR-0890 | Selected credential-free CI double | AR-0888, AR-0889 |
| AR-0891 | Grouped local-inference profiles | AR-0879, AR-0312/0313/0315 |
| AR-0892 | Mock/replay/local/live evidence | AR-0890, AR-0891 |
| AR-0893 | CLI setup/diagnostics | AR-0890, AR-0891, AR-0892, AR-0869/0871/0872 |
| AR-0894 | TUI parity after existing TUI/CI work | AR-0893, AR-0870, AR-0873 |

AR-0894 claims no screen or shortcut before implementation; CLI remains operational.
AR-0874 refreshes docs only after owning ARs finish. Each task publishes exact pins,
hostile cases, labels and unsupported cases. CI denies provider networking and keeps
no secrets. Local jobs cannot silently skip to mocks. Compatibility matrices derive
from executable evidence, not project claims.
