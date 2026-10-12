---
{
  "branch": "feature/ar-1775-configurable-output-router-and-levels",
  "checkpoint_commit": "d7ab95883d3913fae7b1b2646266e114f3de297d",
  "claim_expires": "2026-10-12T22:47:32+00:00",
  "depends_on": [
    "AR-1771"
  ],
  "id": "AR-1775",
  "next_action": "Implement the sole configurable output router, generated project output defaults, and four-level selection after the output contract is accepted.",
  "observed_branch": "feature/ar-1775-configurable-output-router-and-levels",
  "observed_dirty": 0,
  "observed_head": "d7c08c106e52ed9aa144f24ffe130b40dd6c3da7",
  "owner": "ar1775-cli2key-runtime-luna",
  "plan": "../plans/AR-1775-configurable-output-router-and-levels.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1775.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1775.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Implement the mandatory configurable output router and project-configured quiet, normal, verbose, and debug levels.",
  "task_revision": 6,
  "title": "Configurable output router and levels",
  "updated_at": "2026-10-12T20:47:32+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1775-configurable-output-router-and-levels"
}
---

Implement the AR-1771 output contract before progress reporting. Add one typed,
injectable output router which is the mandatory path for all ASB-owned
user-visible output. It must route ordinary human result output and operational
diagnostic/status output to independently configurable safe writers, with stdout
and stderr respectively as the default. Project initialization generates and
persists the `output` configuration with `level = "normal"`; repeated project
initialization preserves an explicit user selection.

Implement the closed levels `quiet`, `normal`, `verbose`, and `debug`, their
documented project configuration and CLI override precedence, and `-q` as a
quiet-level override. Every writer implementation must apply the same level
filter and privacy rules. `debug` is development detail, never credential or
private-host disclosure. Error handling for an unavailable/unwritable configured
destination must be specific, use the AR-1766 diagnostic catalog, and preserve
the original command outcome where safe to do so.

Route all pre-existing ASB output through this router before adding new progress
behavior. JSON is not a configurable human writer: `--json` retains its exact
stdout envelope and silences all human/operational output on both streams.

- 2026-10-12T20:45:06+00:00: AR-1771 dependency is done with accepted contract evidence; promote
  configurable output router implementation.

- 2026-10-12T20:45:28+00:00: Claimed by ar1775-cli2key-runtime-luna.

- 2026-10-12T20:47:24+00:00: Checkpointed source commit d7ab95883d3913fae7b1b2646266e114f3de297d.

- 2026-10-12T20:47:32+00:00: Heartbeat by ar1775-cli2key-runtime-luna.
