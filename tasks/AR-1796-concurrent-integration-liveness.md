---
{
  "branch": "policy/concurrent-integration-liveness",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T14:30:01+00:00",
  "depends_on": [],
  "id": "AR-1796",
  "next_action": "Implement candidate-bound publication admission before moving remote I/O outside the authority lock.",
  "observed_branch": "policy/concurrent-integration-liveness",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "codex-ar1796-integration-liveness",
  "plan": "../plans/AR-1796-concurrent-integration-liveness.md",
  "priority": "P0",
  "schema_version": 1,
  "status": "in_progress",
  "summary": "Keep autonomous development live during concurrent protected-main integration.",
  "task_revision": 4,
  "title": "Concurrent integration liveness",
  "updated_at": "2026-10-11T12:32:25+00:00",
  "worktree_key": ""
}
---

Repair the policy that turns ordinary concurrent protected-main progress into
terminal worker blocking. Preserve exact merge-result assurance while ensuring
workers checkpoint, retry, and continue useful work during integration churn.

- 2026-10-11T12:25:29+00:00: Claimed by codex-ar1796-integration-liveness.

- 2026-10-11T12:30:01+00:00: Heartbeat by codex-ar1796-integration-liveness.

- 2026-10-11T12:32:25+00:00: Independent source audit rejected a naive post-unlock push: it can
  publish a concurrent head. The repair now requires durable candidate identity, serialized
  admission, remote-base fencing, post-push ancestry observation, and explicit ambiguous recovery.
