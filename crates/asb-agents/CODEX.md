# Codex adapter boundary

This module supports the content-pinned Codex CLI 0.153.4 Linux x86_64
executable through the documented non-interactive `codex exec --json`
interface. The exercised executable has SHA-256
`56ef98ab4032d317ab26e9b5e5a175650717351edb16ed9cde0cb6d1734d62da`.

Every attempt uses `--ephemeral`, `--ignore-user-config`,
`--ignore-rules`, strict command-line configuration, and an empty child
environment apart from a minimal system path, isolated state directories, and
a credential-free provider token. Prompts are passed through an unlinked
0600 file descriptor rather than process arguments. Only content-free
lifecycle, tool kind, causal ID, success, and token-count evidence is retained.
Before execution, the adapter requires exact canonical workspace and state
directories with no symlink ancestor. It walks the workspace as a bounded set
of regular files, rejecting symlinks and other filesystem objects, paths over
4 KiB, more than 16,384 entries or 4,096 files, any file over 16 MiB, and more
than 256 MiB in aggregate. This is a preflight against the initial tree, not a
claim of race-free containment against another process mutating that tree.

Provider override is limited to version-negotiated HTTP Responses requests
using an explicit `model_providers.<id>.base_url`, `env_key`,
`wire_api = "responses"`, and `requires_openai_auth = false`. Loopback HTTP
is accepted for deterministic fixtures; non-loopback providers require HTTPS.
The adapter does not claim Codex account/subscription routing, app-server or
exec-server subscriptions, WebSocket Responses, persisted session resume,
native replay, MCP/plugin configuration, web-search correctness, or any
provider/model beyond the explicitly configured endpoint. Those routes must
not be inferred from the cancellation-only extension manifest.
Cancellation signals the unique owned process group and the native fixture
proves its shell child is no longer live. As documented by `asb-runtime`,
daemonized or process-group-escaping descendants require a cgroup-backed
runtime for a hard containment claim; a killed descendant can also remain a
short-lived zombie until its system parent reaps it.

The JSONL decoder is fail-closed for unknown event or item kinds, inconsistent
lifecycle/tool pairing, duplicate JSON members, malformed usage, oversized
lines, and excessive event counts. Raw upstream text, reasoning, commands,
outputs, errors, prompts, credentials, and configuration are deliberately
absent from returned evidence. A SHA-256 pin establishes artifact identity,
not upstream authorship, authenticity, license provenance, or absence of
vulnerabilities.

Interface and provider configuration were checked against the official
[Codex non-interactive documentation](https://developers.openai.com/codex/noninteractive),
[Codex CLI reference](https://developers.openai.com/codex/cli/reference), and
[Codex configuration reference](https://developers.openai.com/codex/config-reference).
