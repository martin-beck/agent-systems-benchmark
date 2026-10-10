---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [
    "AR-1711",
    "AR-1733"
  ],
  "id": "AR-1785",
  "next_action": "Promote after AR-1715 live transport evidence is reconciled; reproduce the provider-plan --use-config and benchmark-live identity mismatch in a clean product-bound worktree.",
  "owner": "",
  "plan": "../plans/AR-1785-openrouter-config-identity-repair.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1785.json",
  "spec_revision": 1,
  "status": "open",
  "summary": "Align persisted OpenRouter configuration, provider-plan selection identity, and benchmark-live selected/all execution.",
  "task_revision": 7,
  "title": "Repair OpenRouter configuration identity for live benchmark qualification",
  "updated_at": "2026-10-10T20:03:00+00:00",
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

- 2026-10-10T20:02:39+00:00: Releasing temporary claim to bind the implementation task to a
  dedicated product branch/worktree before any product command.

- 2026-10-10T20:02:46+00:00: Claimed by ar1785-openrouter-config-repair.

- 2026-10-10T20:02:49+00:00: Recorded command exit 0; command argv SHA-256
  c4ec0e4fce82629a05576c9c1cfaa4587670a9d09fb7ee68f05aa4926c7bad1b.

- 2026-10-10T20:03:00+00:00: Dedicated product branch/worktree created; bind task metadata before
  reclaiming for implementation.
