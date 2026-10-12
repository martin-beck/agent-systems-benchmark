---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": ["AR-1826", "AR-1827"],
  "id": "AR-1828",
  "next_action": "Remove legacy public config, provider-catalog, provider-plan, and ProviderSelection surfaces after setup contains their user-facing selection responsibilities.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1828-retire-provider-selection-surfaces.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {"evidence_class": "contract-test", "evidence_digest": "", "evidence_ref": "", "spec_ref": "specs/AR-1828.json", "spec_revision": 1, "status": "pending"},
  "spec_ref": "specs/AR-1828.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Retire public config/catalog/provider-plan commands and all ProviderSelection artifacts in favor of setup-created explicit config.",
  "task_revision": 1,
  "title": "Retire legacy provider selection and implicit configuration surfaces",
  "updated_at": "2026-10-12T00:00:00Z",
  "worktree_key": "agent-systems-benchmark-ar-1824-retire-provider-selection-surfaces"
}
---

This is an intentional breaking development migration. Provider and model
discovery may remain internal to setup and validation, but no public command,
artifact, flag, schema, help page, or TUI path may retain ProviderSelection or
implicit configuration as a second authority.
