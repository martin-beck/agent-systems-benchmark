---
{
  "schema_version": 1,
  "branch": "feature/ar-1835-authenticated-control-live-isolation-repair",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": ["AR-1763", "AR-1764", "AR-1782"],
  "id": "AR-1835",
  "next_action": "Promote and claim after dependency and ownership review; implement the bounded authenticated control/live-isolation coverage repair.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1835-authenticated-control-live-isolation-repair.md",
  "priority": "P0",
  "spec_acceptance": {"evidence_class": "contract-test", "evidence_digest": "", "evidence_ref": "", "spec_ref": "specs/AR-1835.json", "spec_revision": 1, "status": "pending"},
  "spec_ref": "specs/AR-1835.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Repair the missing authenticated ControlClient/server and delegated live-isolation coverage seam without weakening AR-1783 gates.",
  "task_revision": 1,
  "title": "Authenticated control and live-isolation coverage repair",
  "updated_at": "2026-10-13T00:00:00+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1835-authenticated-control-live-isolation-repair"
}
---

Add a bounded test-only authenticated control fixture and/or delegated-isolation runner for the unresolved AR-1783 coverage seam. Keep all authority, provider, credential, network, privacy and quality gates fail-closed and unchanged.
