---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [
    "AR-1711",
    "AR-1715"
  ],
  "id": "AR-1723",
  "next_action": "Promote after the live runner and provider qualification dependencies are current; implement and qualify the default-online easy run.",
  "owner": "",
  "plan": "../plans/AR-1723-easy-run-default-online.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1723.json",
  "spec_revision": 1,
  "status": "open",
  "summary": "Make the shortest user-facing run use the configured live provider by default while keeping local mock explicit and development authentication warning-only.",
  "task_revision": 2,
  "title": "Make the easy run default to online provider execution",
  "updated_at": "2026-10-11T00:55:34+00:00",
  "worktree_key": ""
}
---

The development prototype must make live provider execution the obvious
default. Missing development credentials are warnings until live execution is
requested; live failures never silently select mock or replay.

- 2026-10-11T00:55:34+00:00: AR-1715 live qualification is accepted at exact main daae06e; promote
  easy-run default-online implementation. No GitHub Reviews API approval object required.
