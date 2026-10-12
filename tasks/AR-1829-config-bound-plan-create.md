---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": ["AR-1826", "AR-1827", "AR-1828"],
  "id": "AR-1829",
  "next_action": "Implement config-only benchmark plan compilation with mandatory --config and --output arguments.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1829-config-bound-plan-create.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {"evidence_class": "contract-test", "evidence_digest": "", "evidence_ref": "", "spec_ref": "specs/AR-1829.json", "spec_revision": 1, "status": "pending"},
  "spec_ref": "specs/AR-1829.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Make asb plan create compile a complete setup file into an explicit-output immutable benchmark plan with no provider-selection inputs.",
  "task_revision": 1,
  "title": "Compile config-bound benchmark plans with explicit output",
  "updated_at": "2026-10-12T00:00:00Z",
  "worktree_key": "agent-systems-benchmark-ar-1829-config-bound-plan-create"
}
---

The normal planning command is exactly `asb plan create --config CONFIG
--output PLAN`. Both paths are mandatory. The plan is the immutable expansion
of the configuration, not a second provider-selection workflow.
