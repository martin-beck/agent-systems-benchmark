# OpenRouter conformance qualification

The pinned free OpenRouter profile is qualified by the credential-free
`openrouter_conformance` and `openrouter_loopback` tests. The matrix covers
OpenCode, OpenDesk, aider, Codex, Qwen Code, Goose, mini-SWE-agent and
OpenHands. Codex uses the pinned Responses route; the other compatible agents
use the pinned Chat Completions route. Gemini is intentionally unsupported.

The tests use loopback and in-process synthetic doubles only. They prove exact
endpoint/model/profile identity, absence of unsupported sampling fields,
credential-boundary rejection, malformed and wrong-model response rejection,
bounded retry and cancellation behavior, stream termination, tool-event
divergence rejection, and explicit replay/live selection. They retain only
digests and typed outcomes; fixture credentials, prompts, endpoint authorities
and response text are never written to evidence.

The qualification does not claim real-provider availability, native execution
of every pinned agent, network-namespace enforcement, or success of a moving
provider model. A live provider contact remains an explicit opt-in operation
and is outside offline CI.
