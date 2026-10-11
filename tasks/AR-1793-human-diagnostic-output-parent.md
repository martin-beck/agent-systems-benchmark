---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T16:55:07+00:00",
  "depends_on": [
    "AR-1784"
  ],
  "id": "AR-1793",
  "next_action": "Repair the legacy human diagnostic output-parent wording contract and add focused positive/negative coverage; do not publish AR-1784 until green.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "codex-ar1793-diagnostic-repair-luna56",
  "plan": "../plans/AR-1793-human-diagnostic-output-parent.md",
  "priority": "P1",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1793.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1793.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Repair the legacy human diagnostic output-parent wording contract exposed by the full ASB CLI suite.",
  "task_revision": 3,
  "title": "Repair human diagnostic output-parent contract",
  "updated_at": "2026-10-11T13:55:07+00:00",
  "worktree_key": ""
}
---

AR-1793 is a narrowly scoped dependent repair for the deterministic human diagnostic contract.

- 2026-10-11: Created from AR-1784 full-suite evidence.
- Baseline origin/main test passes; the AR-1784 head exposed the contract while changing preflight diagnostic callsites.
- Exact failing test: human tests error_presentations_name_cause_state_and_recovery_without_private_text.
- Observed output: No successful state change was reported. where the contract requires a state-change phrase.

- 2026-10-11T13:53:35+00:00: Promote dependency-safe AR-1793 now that AR-1784 is durably done.

- 2026-10-11T13:55:07+00:00: Claimed by codex-ar1793-diagnostic-repair-luna56.
