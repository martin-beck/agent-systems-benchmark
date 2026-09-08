# Provider parity evidence

The cross-agent conformance harness applies one credential-free profile identity
to OpenCode, OpenDesk, aider, Codex, Qwen Code, goose, mini-SWE-agent, and
OpenHands. OpenAI observations validate exact API mode, model, streaming, tool
presence, omitted unsupported sampling fields, and redacted authorization shape.
Ollama observations validate the pinned daemon/model probe and exact translated
profile identity. Codex uses Responses while the other proven routes use Chat
Completions; this route difference is explicit rather than normalized away.

Gemini is unsupported for both profiles because its native Google route cannot
preserve either OpenAI-compatible contract. The harness rejects the complete
selection atomically instead of producing a partial plan.

Replay selection is bound to both agent and provider-profile digests. Parallel
selection, absent choice, unavailable live service, profile mismatch, corrupt
cassette identity, and network-denied replay are tested fail-closed. No prompt,
credential, authorization value, endpoint authority, or host path is retained.

## Evidence limits

| Boundary | Evidence level | Limit |
| --- | --- | --- |
| OpenAI, eight agents | synthetic request verifier | Does not execute pinned agent binaries |
| Ollama, eight agents | synthetic pinned probe and translation | Does not execute the 18.6 GB model |
| Gemini | unsupported | Native Google API is not profile-equivalent |
| Replay/live choice | deterministic in-process conformance | Does not claim network namespace enforcement |
| Platforms | hosted x86_64/aarch64 CI after publication | Native distro matrix remains unqualified |

Accordingly, this harness is structural conformance evidence. Real pinned-agent
wire captures, retry/deadline/cancellation observations, and native platform
qualification remain required before claiming full provider parity.
