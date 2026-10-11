---
{
  "branch": "codex/ar-1792-pty-fixture-determinism",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T02:10:54+00:00",
  "depends_on": [
    "AR-1770",
    "AR-1791"
  ],
  "id": "AR-1792",
  "next_action": "Promote and reproduce the hosted PTY fixture failure in a clean bound worktree; repair the underlying race without serializing or weakening coverage.",
  "observed_branch": "codex/ar-1792-pty-fixture-determinism",
  "observed_dirty": 0,
  "observed_head": "16d8512debab5515068d285fd820e1be0751c51b",
  "owner": "codex-ar1792-pty-repair",
  "plan": "../plans/AR-1792-pty-fixture-determinism.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1792.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Repair the intermittent development broker foreground/restore PTY fixture exposed by PR #555 coverage.",
  "task_revision": 5,
  "title": "Make development broker PTY fixture deterministic",
  "updated_at": "2026-10-11T00:11:18+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1792-pty-fixture-determinism"
}
---

This development repair does not require a GitHub Reviews API approval object.
Independent technical review, exact-head CI, signed DCO integration, and
post-merge verification remain mandatory. It must not turn coverage into a
serial-only or exclusion-based gate.

- 2026-10-11T00:09:38+00:00: Promoted for deterministic PTY fixture repair; no GitHub Reviews API
  approval object is required.

- 2026-10-11T00:09:41+00:00: Claimed by codex-ar1792-pty-repair.

- 2026-10-11T00:10:54+00:00: Heartbeat by codex-ar1792-pty-repair.
