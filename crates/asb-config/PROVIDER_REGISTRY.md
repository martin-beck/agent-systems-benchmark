# Provider registry v1

`ProviderRegistryV1` is the credential-free discovery cache used by the
provider-selection layer. It stores provider and protocol identifiers,
endpoint-identity SHA-256 digests, an optional credential-reference digest, and
qualified model IDs. It never stores endpoint URLs, API keys, response bodies,
or private filesystem paths.

Catalog adapters accept bounded JSON only:

- OpenAI-compatible responses use `data[].id`.
- Gemini responses use `models[].name`; the public `models/` prefix is removed.
- Ollama responses use `models[].name`.

Each response is limited to 64 KiB and 256 models. Unknown fields, malformed
documents, duplicate IDs, zero or stale generations, and oversized payloads
are rejected without replacing the existing cache. Every qualification digest
is recomputed from the model ID during validation, so tampered cache entries
fail closed. Endpoint scheme, credentials, redirects, loopback policy, and
endpoint identity binding are enforced by the authenticated probe boundary in
`asb-agents/src/provider_probe.rs`; the registry intentionally has no endpoint
value to validate.
