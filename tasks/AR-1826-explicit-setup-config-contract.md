---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-13T09:01:59+00:00",
  "depends_on": [],
  "id": "AR-1826",
  "next_action": "Define and approve the one-file explicit setup configuration and plan-binding contract before implementing any command migration.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "ar1826_setup_contract_luna",
  "plan": "../plans/AR-1826-explicit-setup-config-contract.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1826.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1826.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Define the explicit setup-file and config-bound plan contract that replaces implicit user configuration and provider-selection artifacts.",
  "task_revision": 4,
  "title": "Define explicit setup configuration and plan-binding contract",
  "updated_at": "2026-10-13T06:02:05+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1826-explicit-setup-config-contract"
}
---

This is the migration root for a deliberately breaking development CLI route:
`asb setup --output CONFIG`, `asb plan create --config CONFIG --output PLAN`,
then config-bound plan validation, run, and sweep. It replaces implicit XDG or
user-home configuration and all public ProviderSelection artifacts. It does not
implement the commands; it fixes the shared schema, path, secret, freshness,
and compatibility contract on which the remaining ARs depend.

- 2026-10-13T05:54:15+00:00: Opening dependency-free setup contract implementation for assigned
  worker.

- 2026-10-13T06:01:59+00:00: Claimed by ar1826_setup_contract_luna.

- 2026-10-13T06:02:05+00:00: Recorded command exit 0; command argv SHA-256
  9c59abdd443caaa4ce7e0e388ff8b2083eb2af27df45f54ffcbd017cddb53661.
