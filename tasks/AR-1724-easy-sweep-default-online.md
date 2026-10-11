---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [
    "AR-1723",
    "AR-1712"
  ],
  "id": "AR-1724",
  "next_action": "Promote after AR-1723 and recording/comparison dependencies are current; implement and qualify the bounded online sweep.",
  "owner": "",
  "plan": "../plans/AR-1724-easy-sweep-default-online.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1724.json",
  "spec_revision": 1,
  "status": "open",
  "summary": "Extend the simple default-online journey to bounded selected/all agent and workload sweeps with explicit mock and replay alternatives.",
  "task_revision": 2,
  "title": "Make the easy sweep default to online provider execution",
  "updated_at": "2026-10-11T01:38:43+00:00",
  "worktree_key": ""
}
---

This AR keeps the first-run experience short while making selected/all
benchmark coverage straightforward. Development-only authentication and key
management remain warning-only and never block setup.

- 2026-10-11T01:38:43+00:00: Dependencies AR-1723 and AR-1712 are done. Promote easy-sweep
  online-default implementation; --local-mock remains explicit and no GitHub Reviews API approval is
  required.
