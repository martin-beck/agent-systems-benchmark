# Shared OpenAI provider profile

ASB identifies the public OpenAI service only as `https://api.openai.com/v1/`; a custom or
loopback OpenAI-compatible endpoint is a different provider profile. The default model is the
dated `gpt-5.2-2025-12-11` snapshot, not a moving alias. The normalized profile requests reasoning
effort `none`, omits sampling and output-token overrides, and permits one provider request at a
time within explicit byte and deadline bounds.

Credentials are never part of profile evidence. The profile records only an operator-supplied
SHA-256 identity for a logical environment-secret reference, while each adapter translation names
its controlled injection boundary. A digest is not a credential and does not prove that a secret
exists; missing credentials must fail before a real connection attempt.

Codex uses the Responses API. OpenCode, OpenDesk, aider, Qwen Code, Goose, mini-SWE-agent and
OpenHands use Chat Completions with their adapter-specific model spelling. Gemini is unsupported
because its adapter uses the Google GenerateContent protocol. Real account connections remain
opt-in and are never an offline CI requirement. Synthetic request-observation fixtures must prove
effective model, API route, streaming, reasoning, tools, omitted settings, bearer presence and
redaction before this profile is selected atomically across agents. The retained proof contains
only adapter/mode and credential-free profile identity; authorization and prompt content are
neither retained nor hashed.

The model snapshot and API surfaces are documented by OpenAI at
<https://developers.openai.com/api/docs/models/gpt-5.2> and
<https://developers.openai.com/api/docs/guides/latest-model>.
