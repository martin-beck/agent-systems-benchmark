# AI response recording and replay research

Research snapshot: 2026-09-06. These are source/documentation assessments,
not successful compatibility tests. AR-0501 must pin revisions, inspect licenses
and run the same conformance suite before choosing dependencies or importing code.

The pinned-code and bounded-spike assessment is recorded in
[REPLAY_EVALUATION.md](REPLAY_EVALUATION.md). It selects a small safe Rust
implementation and makes no provider or native-platform support claim.

## Literature

[AgentRR: Get Experience from Practice: LLM Agents with Record & Replay](https://arxiv.org/abs/2505.17716)
proposes recording execution experience and replaying it at different abstraction
levels to improve agent efficiency and reliability. Its experience-reuse objective
is related to ASB, but is not evidence that HTTP response replay yields identical
Linux execution timings or cross-agent reasoning trajectories.

[Deterministic Replay for AI Agent Systems](https://arxiv.org/abs/2607.16200)
describes agrepl, transport interception, trace matching and isolated replay.
Its reported fidelity is scoped to the evaluated workloads. Inspect and reproduce
the linked implementation's network isolation and stream handling before adopting
those guarantees. A request/response determinism argument depends on capturing all
relevant external inputs; clock, filesystem, subprocess and concurrency noise remain
outside an LLM-only recorder.

[Application-Integrated Record-Replay of Distributed Systems](https://www2.eecs.berkeley.edu/Pubs/TechRpts/2024/EECS-2024-4.pdf)
is a background reference for replay boundaries and message ordering. The deeper
systems literature should inform causal stream design rather than motivate
replaying the operating system whose performance ASB intends to measure.

## Open-source candidates

| Candidate | Evidence and fit | Required investigation |
| --- | --- | --- |
| [mitmproxy](https://docs.mitmproxy.org/stable/overview/features/) | Mature HTTP capture and server replay; configurable matching | Default response refreshing changes headers; streaming bodies require special capture. Client replay serializes requests and is not a concurrency driver |
| [WireMock](https://wiremock.org/docs/record-playback/) | Standalone proxy recording and generated HTTP stubs | Validate incremental SSE, provider event dialects, sequence matching and pacing; JVM resource footprint |
| [VCR.py](https://vcrpy.readthedocs.io/en/latest/advanced.html) | Custom matchers, cassette filtering and playback controls | Useful Python adapter tests; not a universal interceptor for Rust/Node external clients |
| [LiteLLM mock responses](https://docs.litellm.ai/docs/completion/mock_requests) | Convenient synthetic completion fixtures | Mock responses are not a faithful recording mechanism; distinguish simulation from capture |
| [buffr](https://github.com/RobinBially/buffr) | Documents HTTP/SSE/WebSocket capture and optional original frame pacing | Strong spike candidate; test causal ordering, simultaneous identical calls, backpressure and saturation |
| [llmtape](https://github.com/Ayubjon/llmtape) | LLM-specific proxy and cassettes | Documented model-plus-messages fingerprint omits parameters; insufficient as ASB's strict default matcher |
| [promptecho](https://github.com/shwetank/promptecho) | httpx sync/async and SSE recordings; normalization and filtering | Python-specific boundary; check whether normalization changes wire behavior under measurement |
| [agrepl](https://github.com/taiwrash/agrepl) | Go CLI for transport-level agent replay, accompanying paper | Verify implementation against documentation claims, streaming, certificate behavior and offline failures |
| [acp-core](https://github.com/ky2renzzz/acp-core) | Rust ACP frame recording and divergence analysis | ACP is the host-to-agent boundary, not the agent-to-LLM boundary. Replacing agent frames bypasses measured agent work |
| [llm-d inference simulator](https://github.com/llm-d/llm-d-inference-sim) | GPU-free simulated inference service | Candidate for synthetic latency profiles; does not reproduce a recorded agent answer |
| [rr](https://github.com/rr-debugger/rr) | Linux process recording for debugging | CPU/PMU-dependent support; useful failure diagnosis, inappropriate as the normal systems-performance replay engine |

## Proposed design

Prefer a local provider-compatible response service, configured through each
agent's supported endpoint settings. Do not silently intercept unrelated traffic.
Support OpenAI Chat Completions, OpenAI Responses and Anthropic Messages as
separate dialects; enable each only after conformance tests for the named client.
WebSocket support is a separately tracked capability, never inferred from SSE.

Store immutable versioned cassettes with request method/path, canonical semantic
body, model/options/tools, response status and relevant headers, ordered event
payloads, finish/usage information and monotonic event offsets. Preserve tool-call
identities and causal response references. Hash manifests and payloads; reject
truncated or corrupt captures. Represent transport chunking separately from SSE
event boundaries; neither implies TCP-packet fidelity.

Strict matching includes all semantically relevant request parameters. Any
normalization is explicit, versioned and narrowly scoped to validated volatile
fields. No fuzzy/embedding match in deterministic mode. Divergence, unused required
events, repeated calls beyond policy or absent entries fail closed without outbound
fallback. Responses carrying prior-response IDs require consistent scoped remapping.

Each session/attempt gets its own stream and playback cursor. Identical requests
from parallel sessions cannot consume one another's responses. The same cassette
may be cloned for identical repeated workloads; changed agent versions or different
agents need independently recorded compatible trajectories.

Modes: immediate replay for local overhead; fixed TTFT/event cadence for controlled
experiments; original event pacing for scenario reproduction; seeded synthetic
delay/failure profiles for resilience. Record requested and achieved delays and
replay-service CPU/queueing overhead. An overloaded replay service invalidates
client capacity conclusions.

Record responses at the LLM boundary while the actual agent and engineering tools
execute. Freeze workload input, images, dependencies, locale and seeds. Do not
replay tool execution in a workload-execution benchmark. A separate fully stubbed
diagnostic mode may do so and must have a different result classification.

Raw captures are private by default. Redact credentials in headers, URLs and
bodies before persistence; prompt/output data require explicit fixture review.
Public fixtures are synthetic or separately approved and scanned. Preserve a
consistent redaction mapping without merging distinct requests. Test malicious
payloads, capture size limits and decompression bounds.

## Spike exit criteria

Run every shortlisted tool on one synthetic buffered response, streamed text,
streamed tool call, Responses reference chain, error/retry and mid-stream cancel.
Repeat with identical parallel sessions, reordered requests, offline network denial
and a deliberately unmatched request. Measure replay overhead across concurrency,
verify payload/event fidelity, license compatibility and both native architectures.
Choose reuse, adapter/import, or a small Rust implementation using this evidence;
do not choose a dependency solely because its README says deterministic.
