---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": ["AR-1826"],
  "id": "AR-1827",
  "next_action": "Implement the complete interactive and noninteractive setup authoring route against the explicit configuration contract.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1827-complete-setup-authoring.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {"evidence_class": "contract-test", "evidence_digest": "", "evidence_ref": "", "spec_ref": "specs/AR-1827.json", "spec_revision": 1, "status": "pending"},
  "spec_ref": "specs/AR-1827.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Make asb setup --output the complete authoring route for agents, live providers/models, workloads, monitors, measures, and bounded execution choices.",
  "task_revision": 1,
  "title": "Author complete explicit benchmark configuration with asb setup",
  "updated_at": "2026-10-12T00:00:00Z",
  "worktree_key": "agent-systems-benchmark-ar-1823-complete-setup-authoring"
}
---

`asb setup` becomes the normal user-facing selection and configuration path.
It must write only the requested file and make all supported live and online
choices discoverable without forcing users through separate catalog or
selection commands.
