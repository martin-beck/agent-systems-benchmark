# Local Ollama provider profile

ASB's initial Ollama profile is deliberately local and immutable. It accepts only the numeric
loopback root `http://127.0.0.1:11434/`, probes only `/api/version` and `/api/tags`, and never calls a
model pull, copy, create, or delete endpoint. The exercised Linux x86_64 server is Ollama 0.33.1
with executable SHA-256 `9f595107f966433f93f20ee19043f8e0cdea88e7403672f4dba2cadcb45ee085`.

The selected model is `qwen3-coder:30b`, manifest SHA-256
`06c1097efce0431c2045fe7b2e5108366e43bee1b4603a7aded8f21689e90bca`, Q4_K_M, with the Apache-2.0
license embedded in its manifest. The profile checks the complete model digest, size, format,
family, quantization and context metadata before returning a usable route. A missing model or any
drift fails before agent launch; ASB never pulls implicitly.

OpenCode, OpenDesk, aider, Qwen Code, Goose, mini-SWE-agent and OpenHands use Ollama's
OpenAI-compatible chat endpoint. Codex requires the stateless Responses endpoint. Gemini is
explicitly unsupported because its adapter speaks the Google GenerateContent protocol. The exact
Ollama 0.33.1 API is not claimed on other binaries, architectures, remote endpoints, or model
manifests. Model residency and daemon lifecycle remain operator-managed and are not evidence of
benchmark isolation.
