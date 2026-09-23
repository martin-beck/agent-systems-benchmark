# Provider-aware launches

`provider-plan` is deliberately a side-effect-free selection step. A saved selection becomes
authoritative only when `run` or `sweep` constructs a `ProviderLaunchV1` binding immediately
before creating the durable run. The binding content-addresses the catalog, complete selection,
provider profile, exact agent/adapter route, model/settings, runtime and executable identities,
credential resolver reference, workload/run/attempt identities, and bounded process policy.

The binding is constructor-controlled: the adapter projection must exactly equal every provider,
model, API-mode, settings, and resolver field in the requested launch. Any stale, conflicting,
unknown, or tampered identity fails before run creation or process spawn. The canonical launch
digest is stored in the run definition and terminal report, and is exported to the adapter only as
non-secret `ASB_PROVIDER_*` environment metadata. Credential values never enter argv, ordinary
environment, manifests, artifacts, logs, or errors; the approved resolver boundary remains the
only secret channel.

Every selected agent must have an exact projection. A generic wrapper cannot claim a different
provider by changing its defaults: the launch digest and profile identity are fixed by the
constructor-controlled projection and are revalidated immediately before spawn. Replay remains
offline and must use its existing exact cassette/network-denial contract.

The current experiment plan carries one verified executable identity. Until runtime-bundle
manifests are included in the plan schema, that executable content address is used as the
fail-closed runtime identity; it must match again at snapshot/spawn time. This is an explicit
evidence boundary, not a claim of native third-party provider-service qualification.

## OpenRouter provider matrix (AR-1327)

The OpenRouter profile (AR-1325) is pinned to `https://openrouter.ai/api/v1` with
`ProviderKind::OpenAiCompatible` and the dated model snapshot
`deepseek/deepseek-chat-v3-0324:free@2026-09-22`. Every compatible adapter projection translates
one exact profile: identical endpoint, model, API-mode, settings, and resolver identity; Gemini
is rejected before launch because it has no proven OpenRouter boundary.

| Agent | API mode | Credential target | OpenRouter supported |
| --- | --- | --- | --- |
| OpenCode | ChatCompletions | `OPENROUTER_API_KEY` | Yes |
| OpenDesk | ChatCompletions | `OPENROUTER_API_KEY` | Yes |
| aider | ChatCompletions | `OPENROUTER_API_KEY` | Yes |
| Codex | Responses | `OPENROUTER_API_KEY` | Yes |
| Qwen Code | ChatCompletions | `OPENROUTER_API_KEY` | Yes |
| Goose | ChatCompletions | `OPENROUTER_API_KEY` | Yes |
| mini-SWE-agent | ChatCompletions | `OPENROUTER_API_KEY` | Yes |
| OpenHands | ChatCompletions | `OPENROUTER_API_KEY` | Yes |
| Gemini CLI | — | — | No (rejected atomically) |

`OPENROUTER_API_KEY` is the only secret channel for this family; the environment resolver never
places the value in argv, environment exports, manifests, artifacts, logs, or errors, and the
launch record holds only the credential reference digest. Unsupported or unobservable settings
(for example non-default sampling) remain unsupported and fail closed.

### Egress audit

| Adapter | `openrouter.ai` in known-egress set | Notes |
| --- | --- | --- |
| aider | Yes | Listed since the AR-0315 egress audit; regression-guarded |
| mini-SWE-agent | Yes | Listed since the AR-0315 egress audit; regression-guarded |
| Goose | Yes (added in AR-1327) | Added explicitly, never silently |
| OpenCode / Codex / OpenDesk / Qwen Code / OpenHands | No explicit deny-list | Endpoint passes the generic HTTPS validation; process egress is governed by the runtime sandbox `LoopbackOnly` policy plus the adapter `NO_PROXY` host binding |

No adapter gains a silent egress fallback; every allowance is explicit and covered by a
regression test.
