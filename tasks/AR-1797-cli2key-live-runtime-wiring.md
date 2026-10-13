---
{
  "branch": "feature/ar-1797-cli2key-live-runtime-wiring",
  "checkpoint_commit": "c21b4f5a639e9f4de4bef7c3923cc0cc3f8aa1ea",
  "claim_expires": "2026-10-13T02:32:04+00:00",
  "depends_on": [
    "AR-1729",
    "AR-1731",
    "AR-1732",
    "AR-1733",
    "AR-1779"
  ],
  "id": "AR-1797",
  "next_action": "Wire the verified project-local cli2key bridge into the runtime-owned sidecar launch path, then qualify fake and opt-in live benchmark execution without exposing credentials.",
  "observed_branch": "DETACHED",
  "observed_dirty": 0,
  "observed_head": "da0cbe0848a10e61a2cd0b9a3144217842a3369b",
  "owner": "ar1797-cli2key-luna",
  "plan": "../plans/AR-1797-cli2key-live-runtime-wiring.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "environmental",
    "evidence_digest": "sha256:b6e14cca735fb83ad65fa8f18571d2414c446826dc6eedbb7e8c1f55f2a735d9",
    "evidence_ref": "main/da0cbe0848a10e61a2cd0b9a3144217842a3369b",
    "spec_ref": "specs/AR-1797.json",
    "spec_revision": 1,
    "status": "pass"
  },
  "spec_ref": "specs/AR-1797.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Repair cli2key so an installed pinned bridge receives a runtime-issued sidecar launch and can execute a development ASB benchmark.",
  "task_revision": 7,
  "title": "Wire cli2key sidecar into live benchmark execution",
  "updated_at": "2026-10-13T00:35:10+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1797-cli2key-live-runtime-wiring"
}
---

AR-1729 supplies the sidecar lifecycle and AR-1731 through AR-1733 supply the
adapter, orchestration, and qualification contracts, but direct CLI live
execution still rejects cli2key because no runtime-issued sidecar launch reaches
the execution path. Repair that integration. After AR-1779 installs the pinned
bridge through `asb tool install cli2key`, resolve only its verified receipt,
start one runtime-owned loopback sidecar per run or bounded sweep, and provide
the resulting ephemeral launch only to the Codex Responses adapter.

The user-controlled Codex login remains outside ASB: do not read, copy, log, or
persist its authentication material. Do not route through Codex app-server,
fall back to OpenRouter, API keys, replay, or another provider, or describe the
development bridge as official OpenAI support. A credential-free fake is required
in CI; one opt-in user-authorized live development benchmark proves the complete
path and records only redacted provenance and typed outcome evidence.

- 2026-10-13T00:28:58+00:00: Dependencies AR-1729, AR-1731, AR-1732, AR-1733 and AR-1779 are done;
  promote for dependency-safe cli2key runtime wiring in parallel with blocked TUI repair.

- 2026-10-13T00:32:04+00:00: Claimed by ar1797-cli2key-luna.

- 2026-10-13T00:33:37+00:00: Recorded command exit 0; command argv SHA-256
  05423f546638fa0266e03f759296fdc2689e698f112d4b7b172bb75e95f5dca1.

- 2026-10-13T00:35:05+00:00: Checkpointed source commit c21b4f5a639e9f4de4bef7c3923cc0cc3f8aa1ea.

- 2026-10-13T00:35:10+00:00: PR #603 merged cli2key runtime authority wiring. Exact protected main
  descendant was independently reviewed clean; credential-free fake cli2key contract passed; focused
  runtime (14) and CLI (4), full workspace tests, fmt, clippy, and all recorded PR #603 hosted
  checks passed. No opt-in live benchmark was run because user authorization and Codex login were
  unavailable; no credentials were read or exposed.
