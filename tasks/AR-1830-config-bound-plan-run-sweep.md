---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": ["AR-1828", "AR-1829"],
  "id": "AR-1830",
  "next_action": "Require explicit config validation for plan validation, live run, and live sweep, then wire bounded multi-agent execution.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1830-config-bound-plan-run-sweep.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {"evidence_class": "contract-test", "evidence_digest": "", "evidence_ref": "", "spec_ref": "specs/AR-1830.json", "spec_revision": 1, "status": "pending"},
  "spec_ref": "specs/AR-1830.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Make plan validation, run, and sweep require the setup config and safely execute complete live benchmark selections from config-bound plans.",
  "task_revision": 1,
  "title": "Execute config-bound live runs and sweeps",
  "updated_at": "2026-10-12T00:00:00Z",
  "worktree_key": "agent-systems-benchmark-ar-1830-config-bound-plan-run-sweep"
}
---

The plan path must always be accompanied by `--config`. This AR makes that
pair the only execution input for validation, one-point live runs, and bounded
multi-agent/model/workload sweeps.
