---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": ["AR-1711", "AR-1733"],
  "id": "AR-1789",
  "next_action": "Promote and claim; reproduce the asb-metrics full-workspace-only failure before changing product code.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1789-metrics-fixture-determinism.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1789.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Repair nondeterministic asb-metrics malformed-configuration fixture failure exposed by AR-1788 review.",
  "task_revision": 1,
  "title": "Make metrics malformed-configuration fixture deterministic",
  "worktree_key": "agent-systems-benchmark-ar1789-metrics-fixture-determinism",
  "updated_at": "2026-10-10T21:31:00+00:00"
}
---

This development repair does not require a GitHub Reviews API approval object.
Independent technical review, exact-head CI, signed DCO integration, and
post-merge verification remain mandatory. It must preserve typed rejection and
the ordinary parallel workspace gate.

- 2026-10-10T21:31:00Z: Created from AR-1788 review. The exact candidate's
  ordinary workspace failed in `kernel::tests::missing_malformed_and_unsafe_configuration_fail_closed`
  while the isolated test passed. No publication or merge is authorized until
  the cross-test contamination is repaired.
