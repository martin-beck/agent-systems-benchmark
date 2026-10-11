---
{
  "branch": "codex/ar-1792-pty-fixture-determinism",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": ["AR-1770", "AR-1791"],
  "id": "AR-1792",
  "next_action": "Promote and reproduce the hosted PTY fixture failure in a clean bound worktree; repair the underlying race without serializing or weakening coverage.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1792-pty-fixture-determinism.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1792.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Repair the intermittent development broker foreground/restore PTY fixture exposed by PR #555 coverage.",
  "task_revision": 1,
  "title": "Make development broker PTY fixture deterministic",
  "updated_at": "2026-10-11T00:05:00+00:00",
  "worktree_key": ""
}
---

This development repair does not require a GitHub Reviews API approval object.
Independent technical review, exact-head CI, signed DCO integration, and
post-merge verification remain mandatory. It must not turn coverage into a
serial-only or exclusion-based gate.
