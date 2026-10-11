---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T01:55:40+00:00",
  "depends_on": [
    "AR-1711",
    "AR-1715"
  ],
  "id": "AR-1723",
  "next_action": "Promote after the live runner and provider qualification dependencies are current; implement and qualify the default-online easy run.",
  "owner": "codex-ar1723-online-default",
  "plan": "../plans/AR-1723-easy-run-default-online.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1723.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Make the shortest user-facing run use the configured live provider by default while keeping local mock explicit and development authentication warning-only.",
  "task_revision": 3,
  "title": "Make the easy run default to online provider execution",
  "updated_at": "2026-10-11T00:55:40+00:00",
  "worktree_key": ""
}
---

The development prototype must make live provider execution the obvious
default. Missing development credentials are warnings until live execution is
requested; live failures never silently select mock or replay.

- 2026-10-11T00:55:34+00:00: AR-1715 live qualification is accepted at exact main daae06e; promote
  easy-run default-online implementation. No GitHub Reviews API approval object required.

- 2026-10-11T00:55:40+00:00: Claimed by codex-ar1723-online-default.
