---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-10T22:02:21+00:00",
  "depends_on": [
    "AR-1711",
    "AR-1733"
  ],
  "id": "AR-1785",
  "next_action": "Promote after AR-1715 live transport evidence is reconciled; reproduce the provider-plan --use-config and benchmark-live identity mismatch in a clean product-bound worktree.",
  "owner": "ar1785-openrouter-config-repair",
  "plan": "../plans/AR-1785-openrouter-config-identity-repair.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1785.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Align persisted OpenRouter configuration, provider-plan selection identity, and benchmark-live selected/all execution.",
  "task_revision": 3,
  "title": "Repair OpenRouter configuration identity for live benchmark qualification",
  "updated_at": "2026-10-10T20:02:21+00:00",
  "worktree_key": ""
}
---

This repair is development-only and must not require a GitHub Reviews API
approval object. Independent technical review, exact-head CI, signed DCO
merging, and post-merge verification remain required.

- 2026-10-10T20:02:18+00:00: AR-1715 live transport succeeded for selected and all agents, but
  provider-plan --use-config and benchmark-live selection identity fail closed. Start the focused
  configuration identity repair.

- 2026-10-10T20:02:21+00:00: Claimed by ar1785-openrouter-config-repair.
