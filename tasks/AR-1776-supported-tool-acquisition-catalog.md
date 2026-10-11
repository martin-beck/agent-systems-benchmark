---
{
  "branch": "feature/ar-1776-supported-tool-acquisition-catalog",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T08:27:35+00:00",
  "depends_on": [
    "AR-1761",
    "AR-1762"
  ],
  "id": "AR-1776",
  "next_action": "Define the authoritative supported executable-tool acquisition catalog, official primary sources, binary/source alternatives, verification identities, and build recipes.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "codex-ar1776-tool-catalog-luna56",
  "plan": "../plans/AR-1776-supported-tool-acquisition-catalog.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1776.json",
    "spec_revision": 2,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1776.json",
  "spec_revision": 2,
  "status": "in_progress",
  "summary": "Define pinned official acquisition and build recipes for every ASB-supported executable tool.",
  "task_revision": 10,
  "title": "Supported-tool acquisition catalog",
  "updated_at": "2026-10-11T06:29:42+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1776-supported-tool-acquisition-catalog"
}
---

Replace the current local-file-only installer input with an authoritative,
versioned acquisition catalog for every ASB-supported agent, harness, benchmark,
and support tool. Installable workload bundles belong exclusively to the separate
`asb workload` series. Each tool entry names its official
primary distribution/source, supported platform tuple, exact version/revision,
immutable artifact/source identity, digest and signature/provenance verifier when
available, executable entrypoint, license, and one or more bounded
build recipes. It must distinguish a reusable verified system binary, an
official prebuilt artifact, and a source-build fallback.

The catalog is the sole authority behind `asb tool install <supported-id>`;
the default command must not require a user-supplied local source. It may use a
locally present compatible tool only after it validates the same identity and
records that reuse. Unsupported targets, unavailable official artifacts, missing
verification material, incompatible licenses, and unavailable build recipes are
typed actionable outcomes, not a reason to invoke arbitrary package managers or
shell installers. Keep credentials out of records.

The initial catalog inventory must cover the supported executable-agent IDs
`aider`, `codex`, `gemini`, `goose`, `mini-swe`, `opencode`, `opendesk`,
`openhands`, `openjiuwen`, and `qwen-code`; the development-only `cli2key`
bridge/sidecar when enabled; and optional monitoring executables `perf` and
`bpftool`. It must distinguish provider APIs (OpenAI, OpenRouter, Ollama) from
installable executables. Runtime host capabilities such as Linux, cgroup v2,
user systemd delegation, and Bubblewrap are declared by the companion command
dependency/preflight catalog rather than falsely represented as downloadable
tools.

- 2026-10-11T06:25:21+00:00: dependencies verified

- 2026-10-11T06:25:30+00:00: Claimed by codex-ar1776-tool-catalog-luna56.

- 2026-10-11T06:27:02+00:00: Recorded command exit 1; command argv SHA-256
  a9dbdcac7cd706eebbafcd3cc86d85708ca25d0df1fc27f3b3bc2da8385daae0.

- 2026-10-11T06:27:06+00:00: Recorded command exit 1; command argv SHA-256
  1d96f512ae699afbb6c512f04aca299d54e0f23c55966ae09f00de84476ab946.

- 2026-10-11T06:27:15+00:00: Recorded command exit 0; command argv SHA-256
  b086d913d703881849313a9683ca8c6ccc15d9d8bdba2668d9f37be1fb35d81f.

- 2026-10-11T06:27:35+00:00: Heartbeat by codex-ar1776-tool-catalog-luna56.

- 2026-10-11T06:28:33+00:00: Recorded command exit 101; command argv SHA-256
  6ab752c743a2104a0d04dfacc5323060ecf9f4afaac0448a6dc52830788c517b.

- 2026-10-11T06:29:42+00:00: Recorded command exit 101; command argv SHA-256
  6ab752c743a2104a0d04dfacc5323060ecf9f4afaac0448a6dc52830788c517b.
