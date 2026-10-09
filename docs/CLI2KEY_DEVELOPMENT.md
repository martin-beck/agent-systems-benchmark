# Development cli2key contract

`cli2key` is ASB's name for an explicit development-only, unofficial loopback
bridge from a user-approved Codex OAuth session to a bounded OpenAI-compatible
Responses client. It does not create an OpenAI Platform API key. The key supplied
to an ASB child is a fresh random credential that authenticates only that child
to the local proxy invocation.

The machine-readable contract is
[`config/cli2key-bridge-v1.json`](../config/cli2key-bridge-v1.json). This decision
freezes `CodingForMoney/codex-bridge` v0.1.10 at source commit
`da5d271db08cca1f4666c1b035dbfb6c6f9b8f47`, tree
`682881f6d7fea1fa117bf0899ae0754291bba3c4`, MIT, and GitHub source archive
SHA-256 `db256f8b8b9392835fb1277cadcff6ded3da99a6831f3ff902d5f7721cc7d5bb`
(54,114 bytes). ASB resolves the tag once to these immutable identities and never
follows a moving tag, branch, package, or container.

## Why this bridge

The pinned source exposes authenticated `GET /v1/models` and
`POST /v1/responses`, defaults to `127.0.0.1`, permits an explicit bind override,
generates a separate 32-byte bridge client key, provides a documented `key
refresh` command, reads a fresh upstream credential snapshot for every request,
never writes or refreshes the Codex login, and has bounded body and retry
configuration. Its advertised model list is a bridge compatibility list rather
than a claim to mirror an official platform catalog.

No evaluated bridge met every preference. `dvcrn/codex-oauth-proxy` 1.1.0 imports
into its own store but lacks downstream client authentication and listens on all
interfaces; its later unreleased source adds client authentication but retains
the wildcard listener. `LudwigAJ/codex-proxy` has the right loopback and client
authentication boundaries, but couples its generated key to the persistent OAuth
store and exposes no supported per-invocation rotation interface. The selected
bridge instead uses documented `CODEX_HOME` credential selection plus `key
refresh`; it does not import OAuth into its own store. That preference is recorded
as unmet rather than hidden. ASB never parses, copies, writes, or refreshes the
Codex authentication file; only the bridge reads the explicitly selected login.

## Frozen boundary

- The bridge process must bind only an exact numeric loopback address. A wildcard,
  hostname, private-network, or public bind fails closed.
- The runtime must create an invocation-private bridge home, invoke the supported
  `codex-bridge key refresh` operation there, capture the fresh random local key
  through a private inherited channel, and remove staged state during bounded
  teardown. The later lifecycle AR owns that implementation.
- ASB passes no OAuth material and never opens an authentication file. It may
  select a user-approved logical Codex home for the bridge; raw private paths and
  credential contents never enter plans, argv, logs, or evidence. Login and token
  refresh remain user-controlled Codex operations and never run automatically in
  CI.
- Discovery accepts at most 128 unique model IDs and 256 KiB. Exactly one
  non-streaming Responses request may return at most 1 MiB within 60 seconds.
- No fallback to a platform key, another provider, replay, or a different bridge
  revision is permitted.
- Evidence is labeled `development-only-unofficial`. Authentication, signature,
  and key-management gaps remain visible warnings but do not block this bounded
  development path; they provide no production authorization.

Codex `app-server` is an agent control protocol, not a raw model provider. Routing
another benchmarked agent through it would create nested-agent execution, mix
tool and policy behavior into the model boundary, and invalidate normal benchmark
comparability. That route is prohibited by this contract.

## Evidence and failures

The credential-free fake and opt-in live probe live in
[`tools/cli2key-spike`](../tools/cli2key-spike/README.md). Both prove bounded model
discovery and one Responses exchange. The report retains only the immutable bridge
identity, evidence class, selected model identity, bounded counts, pass/fail shape,
and fixed warnings. OAuth tokens, local keys, authorization headers, prompts,
provider bodies, private paths, and raw exception text are neither retained nor
hashed.

Failures use the closed taxonomy in the machine contract: invalid contract or
opt-in, non-loopback endpoint, missing private inputs, connection/deadline,
authentication, discovery limit/shape/model selection, and Responses
status/size/shape. Diagnostics expose only the stable failure code.

This freeze is sufficient for later runtime, provider, adapter, run/sweep, and
qualification ARs to build against. It does not claim those later paths already
exist, and it is not official OpenAI, production, provider-authority, native
platform, or release evidence.
