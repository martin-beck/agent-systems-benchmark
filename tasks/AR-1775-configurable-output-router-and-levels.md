---
{
  "branch": "feature/ar-1775-configurable-output-router-and-levels",
  "checkpoint_commit": "d7ab95883d3913fae7b1b2646266e114f3de297d",
  "claim_expires": "2026-10-12T22:49:36+00:00",
  "depends_on": [
    "AR-1771"
  ],
  "id": "AR-1775",
  "next_action": "Implement the sole configurable output router, generated project output defaults, and four-level selection after the output contract is accepted.",
  "observed_branch": "feature/ar-1775-configurable-output-router-and-levels",
  "observed_dirty": 1,
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
  "task_revision": 16,
  "title": "Configurable output router and levels",
  "updated_at": "2026-10-12T20:53:18+00:00",
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

- 2026-10-12T20:48:18+00:00: Heartbeat by ar1775-cli2key-runtime-luna.

- 2026-10-12T20:49:13+00:00: Recorded command exit 0; command argv SHA-256
  05423f546638fa0266e03f759296fdc2689e698f112d4b7b172bb75e95f5dca1.

- 2026-10-12T20:49:36+00:00: Heartbeat by ar1775-cli2key-runtime-luna.

- 2026-10-12T20:49:45+00:00: Recorded command exit 0; command argv SHA-256
  05faae2b09a181ad08a6c6862f1c232e689dee071f41e897288fcb13981a49cf.

- 2026-10-12T20:50:50+00:00: Recorded command exit 0; command argv SHA-256
  8acca2e00c96fbcd7f4fd47063e1d51d6bd90ee8831fa0bc8312cb3323ac2812.

- 2026-10-12T20:51:29+00:00: Recorded command exit 0; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-12T20:52:01+00:00: Recorded command exit 101; command argv SHA-256
  1024fc1e6cefa31b078f0b5d2ed85c7ea4586897b388bf1e308f62db58574461.

- 2026-10-12T20:52:56+00:00: Recorded command exit 0; command argv SHA-256
  53bb3b1941ff1516c3e43d286551fda4447292c217edea1208a3b18886a9611a.

- 2026-10-12T20:53:18+00:00: Recorded command exit 1; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.
