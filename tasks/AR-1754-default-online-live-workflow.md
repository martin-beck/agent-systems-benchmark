---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T02:47:38+00:00",
  "depends_on": [
    "AR-1699",
    "AR-1700",
    "AR-1723",
    "AR-1724",
    "AR-1733"
  ],
  "id": "AR-1754",
  "next_action": "Promote after AR-1723, AR-1724, and cli2key qualification AR-1733 are current; inventory every run/sweep/easy/record/campaign/TUI entry point, implement default-online routing with explicit --local-mock and offline replay alternatives, then qualify positive and negative paths.",
  "owner": "codex-ar1754-online-integration",
  "plan": "../plans/AR-1754-default-online-live-workflow.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1754.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Make configured online/live provider execution the default for canonical run, sweep, easy, recording/campaign, and TUI handoff paths while keeping local mock and strict replay explicit alternatives.",
  "task_revision": 4,
  "title": "Make online execution the default across ASB workflows",
  "updated_at": "2026-10-11T02:17:46+00:00",
  "worktree_key": ""
}
---

The user-facing ASB workflow currently requires explicit online/live flags in
several places and makes `asb easy run|sweep` local-mock-only. Define one
consistent development contract: configured online provider execution is the
default, while `--local-mock` is an explicit credential-free alternative and
strict offline replay remains explicit. A live failure must remain a typed
failure; it must never silently fall back to mock or replay.

Cover canonical `run`, `sweep`, `benchmark-live`, `record-live`, recording
campaigns, `asb easy` commands, and the ASB/TUI handoff/configuration contract.
Do not modify the separate asb-tui repository in this AR; publish any required
cross-repository contract and dependency notes in the state plan. Preserve
credential-reference-only configuration, bounded progress, network/cost
warnings, provider/model/agent/workload identity, and all existing fail-closed
production authority checks.

- 2026-10-11T02:17:31+00:00: Dependencies AR-1699, AR-1700, AR-1723, AR-1724, and AR-1733 are
  released/current; promote for canonical online-default integration.

- 2026-10-11T02:17:38+00:00: Claimed by codex-ar1754-online-integration.

- 2026-10-11T02:17:46+00:00: Recorded command exit 0; command argv SHA-256
  904023b98f9c92c714e92042ea44849c3dc5cd79aee9bc2c4f6c2da4145ad191.
