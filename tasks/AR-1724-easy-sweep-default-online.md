---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T02:08:46+00:00",
  "depends_on": [
    "AR-1723",
    "AR-1712"
  ],
  "id": "AR-1724",
  "next_action": "Promote after AR-1723 and recording/comparison dependencies are current; implement and qualify the bounded online sweep.",
  "owner": "codex-ar1724-online-sweep",
  "plan": "../plans/AR-1724-easy-sweep-default-online.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1724.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Extend the simple default-online journey to bounded selected/all agent and workload sweeps with explicit mock and replay alternatives.",
  "task_revision": 4,
  "title": "Make the easy sweep default to online provider execution",
  "updated_at": "2026-10-11T01:38:52+00:00",
  "worktree_key": ""
}
---

This AR keeps the first-run experience short while making selected/all
benchmark coverage straightforward. Development-only authentication and key
management remain warning-only and never block setup.

- 2026-10-11T01:38:43+00:00: Dependencies AR-1723 and AR-1712 are done. Promote easy-sweep
  online-default implementation; --local-mock remains explicit and no GitHub Reviews API approval is
  required.

- 2026-10-11T01:38:46+00:00: Claimed by codex-ar1724-online-sweep.

- 2026-10-11T01:38:52+00:00: Recorded command exit 0; command argv SHA-256
  7e3fd5fe835fd95a2f3e78b0f0f8360951d32181f945987a156b545fe37a699f.
