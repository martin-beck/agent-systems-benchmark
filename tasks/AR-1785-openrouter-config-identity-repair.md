---
{
  "id": "AR-1785",
  "title": "Repair OpenRouter configuration identity for live benchmark qualification",
  "priority": "P0",
  "depends_on": ["AR-1711", "AR-1733"],
  "plan": "../plans/AR-1785-openrouter-config-identity-repair.md",
  "summary": "Align persisted OpenRouter configuration, provider-plan selection identity, and benchmark-live selected/all execution.",
  "status": "planned",
  "next_action": "Promote after AR-1715 live transport evidence is reconciled; reproduce the provider-plan --use-config and benchmark-live identity mismatch in a clean product-bound worktree.",
  "owner": "",
  "claim_expires": "",
  "checkpoint_commit": "",
  "branch": "",
  "worktree_key": "",
  "schema_version": 1,
  "spec_ref": "specs/AR-1785.json",
  "spec_revision": 1,
  "task_revision": 1,
  "updated_at": "2026-10-10T20:01:00+00:00"
}
---

This repair is development-only and must not require a GitHub Reviews API
approval object. Independent technical review, exact-head CI, signed DCO
merging, and post-merge verification remain required.
