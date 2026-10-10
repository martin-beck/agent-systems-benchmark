---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T00:40:24+00:00",
  "depends_on": [
    "AR-1770",
    "AR-1788",
    "AR-1789",
    "AR-1790"
  ],
  "id": "AR-1791",
  "next_action": "Bind an isolated current-main worktree, implement and qualify both fixture repairs, then independently review, publish exact-head CI, merge, and requalify AR-1785.",
  "owner": "codex-ar1791-fixture-repair",
  "plan": "../plans/AR-1791-tui-fixture-isolation.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1791.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Make the GCC descendant-marker and rustup descriptor-cleanup TUI fixtures deterministic under repeated ordinary workspace execution.",
  "task_revision": 3,
  "title": "Repair recurrent TUI fixture isolation failures",
  "updated_at": "2026-10-10T23:10:24+00:00",
  "worktree_key": ""
}
---

This development repair does not require a GitHub Reviews API approval object. Independent technical review, exact-head CI, signed DCO integration, and post-merge verification remain mandatory.

- 2026-10-10T23:10:24+00:00: Clean state worktree permits promotion; dependencies complete. Start
  fixture isolation repair under full development gates, with no GitHub Reviews API approval object.

- 2026-10-10T23:10:24+00:00: Claimed by codex-ar1791-fixture-repair.
