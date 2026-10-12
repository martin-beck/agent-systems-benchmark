---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-12T23:54:29+00:00",
  "depends_on": [
    "AR-1689"
  ],
  "id": "AR-1833",
  "next_action": "Promote and claim after AR-1689 receipt repair is authorized.",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "ar1689-tui-qualification-luna",
  "plan": "../plans/AR-1833-repair-ar1689-receipt.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "journey",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1833.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1833.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Repair malformed AR-1689 paired qualification evidence before relying on its done state.",
  "task_revision": 3,
  "title": "Repair AR-1689 receipt bytes",
  "updated_at": "2026-10-12T21:54:29+00:00",
  "worktree_key": ""
}
---

Repair AR-1689’s malformed receipt bytes without changing the reviewed paired qualification content.

- 2026-10-12T21:54:18+00:00: Promote P0 evidence repair because AR-1689 receipt bytes were malformed
  after release.

- 2026-10-12T21:54:29+00:00: Claimed by ar1689-tui-qualification-luna.
