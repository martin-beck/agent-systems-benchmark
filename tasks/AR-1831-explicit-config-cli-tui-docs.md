---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": ["AR-1827", "AR-1828", "AR-1829", "AR-1830"],
  "id": "AR-1831",
  "next_action": "Align CLI help, human output, completions, TUI handoff, examples, and migration documentation to the explicit setup-config route.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "",
  "plan": "../plans/AR-1831-explicit-config-cli-tui-docs.md",
  "priority": "P1",
  "schema_version": 1,
  "spec_acceptance": {"evidence_class": "contract-test", "evidence_digest": "", "evidence_ref": "", "spec_ref": "specs/AR-1831.json", "spec_revision": 1, "status": "pending"},
  "spec_ref": "specs/AR-1831.json",
  "spec_revision": 1,
  "status": "planned",
  "summary": "Make every CLI, TUI handoff, help page, completion, and example teach the explicit setup-output and plan-config workflow.",
  "task_revision": 1,
  "title": "Align CLI, TUI handoff, and documentation to explicit config workflow",
  "updated_at": "2026-10-12T00:00:00Z",
  "worktree_key": "agent-systems-benchmark-ar-1827-explicit-config-cli-tui-docs"
}
---

The command migration is incomplete until users cannot be directed toward
removed catalog, config, or provider-selection commands by help text, shell
completion, generated guides, or the optional standalone TUI.
