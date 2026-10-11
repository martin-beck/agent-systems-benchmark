---
{
  "branch": "repair/ar-1794-protected-main-merge-path",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T15:22:16+00:00",
  "depends_on": [
    "AR-1781"
  ],
  "id": "AR-1794",
  "next_action": "Keep open pending a clean signed local merge and green exact-main provenance receipt; do not use GitHub web merge.",
  "observed_branch": "repair/ar-1794-protected-main-merge-path",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "ar1794-protected-merge-repair-luna56",
  "plan": "../plans/AR-1794-protected-main-merge-path-repair.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "hosted",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1794.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1794.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Repair the protected-main publication path after a GitHub web merge produced a rejected committer identity.",
  "task_revision": 2,
  "title": "Protected-main merge-path repair",
  "updated_at": "2026-10-11T13:22:16+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1794-protected-main-merge-path"
}
---

Repair the merge publication path without weakening portable provenance or
rewriting protected main. The prior AR-1781 merge and failed exact-main run are
inputs, not disposable evidence.

- 2026-10-11T13:22:16+00:00: Claimed by ar1794-protected-merge-repair-luna56.
