---
{
  "branch": "feature/ar-1784-command-dependency-registry-and-preflight",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T10:10:16+00:00",
  "depends_on": [
    "AR-1776"
  ],
  "id": "AR-1784",
  "next_action": "Independent exact diff review is complete; full asb-cli suite has one pre-existing human diagnostic assertion failure unrelated to AR-1784, while all four new registry tests and public-command inventory test pass. Parent should decide PR publication with this known baseline blocker.",
  "observed_branch": "feature/ar-1784-command-dependency-registry-and-preflight",
  "observed_dirty": 6,
  "observed_head": "fe7f1ce8b34d286ae7e0a83d1d64b2211e7302a5",
  "owner": "ar1784_dependency_registry_luna56",
  "plan": "../plans/AR-1784-command-dependency-registry-and-preflight.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1784.json",
    "spec_revision": 2,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1784.json",
  "spec_revision": 2,
  "status": "in_progress",
  "summary": "Make every ASB command dependency declared, installable or explicitly host-only, and fail-fast before a missing-tool runtime failure.",
  "task_revision": 43,
  "title": "Command dependency registry and preflight",
  "updated_at": "2026-10-11T08:47:16+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1784-command-dependency-registry-and-preflight"
}
---

Create a generated, typed registry that maps every public ASB command, adapter,
monitor, runtime path, and source-build recipe to the exact executable tools,
workload bundles, libraries, and host capabilities it needs. A dependency is
either an `asb tool install` supported ID, an `asb workload install` supported
ID, a catalog-pinned project build dependency, or an explicitly host-only
capability with a concrete platform remediation. No command may invoke or assume
an undeclared executable.

The initial required executable IDs are: `aider`, `codex`, `gemini`, `goose`,
`mini-swe`, `opencode`, `opendesk`, `openhands`, `openjiuwen`, `qwen-code`,
development-only `cli2key`, optional `perf`, and optional `bpftool`. Provider
APIs are not installable tools. Core procfs/cgroup collection needs no external
binary. Linux/cgroup-v2/user-systemd/Bubblewrap and kernel permission/BTF/PMU
conditions are host capabilities, not pretend downloads; preflight must report
them before execution with the owner/action needed to repair them.

Expose an `asb` preflight/status view that reports readiness, missing supported
IDs with exact install commands, missing project-local dependencies, and
uninstallable host-only conditions before a run starts. Generate coverage from
authoritative dispatch/adapter/build/workload declarations, so adding future
tools or workloads requires catalog, command-dependency, preflight, selection,
and end-to-end development-run tests in the same change.

- 2026-10-11T08:10:08+00:00: AR-1776 is complete and its tool catalog is available; promote
  dependency-ready AR-1784 for implementation.

- 2026-10-11T08:10:11+00:00: Claimed by ar1784_dependency_registry_luna56.

- 2026-10-11T08:10:16+00:00: Heartbeat by ar1784_dependency_registry_luna56.

- 2026-10-11T08:10:39+00:00: Recorded command exit 0; command argv SHA-256
  431771e25c6fcb63cee2c212739b3c89638fc0e4eea3b496d3f3b0603f464748.

- 2026-10-11T08:11:05+00:00: Recorded command exit 0; command argv SHA-256
  a80c3db7ed242b537b65e8f3b4a3ddb2554300ce2b1a5e4f8d3ac0651f3d0869.

- 2026-10-11T08:12:21+00:00: Recorded command exit 0; command argv SHA-256
  c7323847342774673319b7ffd857e626b77892358d868ecb570bff056d311898.

- 2026-10-11T08:12:46+00:00: Recorded command exit 0; command argv SHA-256
  ebd2395a6f3af72a541d6b9dd8ffecb9ec9085e6f64d76d108293c9c622d6a56.

- 2026-10-11T08:13:14+00:00: Recorded command exit 101; command argv SHA-256
  0e8b8ec438821ebf3b5b185af8adff8c05c54025dba46010f1057a05ba1d6d2b.

- 2026-10-11T08:13:41+00:00: Recorded command exit 0; command argv SHA-256
  c7323847342774673319b7ffd857e626b77892358d868ecb570bff056d311898.

- 2026-10-11T08:14:06+00:00: Recorded command exit 0; command argv SHA-256
  a4eb05d6d82c911062a7e91d765c0bcdc391fb7aa6781e391d86d09d8ae4a11b.

- 2026-10-11T08:14:33+00:00: Recorded command exit 0; command argv SHA-256
  47bcc84d3b9fdd5b3bb1e5ae460b912d8bf60595e58088926ab03e366e708695.

- 2026-10-11T08:14:49+00:00: Recorded command exit 101; command argv SHA-256
  0e8b8ec438821ebf3b5b185af8adff8c05c54025dba46010f1057a05ba1d6d2b.

- 2026-10-11T08:15:24+00:00: Recorded command exit 0; command argv SHA-256
  c7323847342774673319b7ffd857e626b77892358d868ecb570bff056d311898.

- 2026-10-11T08:15:54+00:00: Recorded command exit 101; command argv SHA-256
  0e8b8ec438821ebf3b5b185af8adff8c05c54025dba46010f1057a05ba1d6d2b.

- 2026-10-11T08:16:37+00:00: Root cause of repeated exit 101: preflight was added as an
  InvocationKind while an accidental stale CommandKind::Preflight arm remained after family mapping
  to Doctor. Remove stale arm and rerun cargo check. Wrapper post-reconcile also repeatedly saw
  disposable handoffctl pycache size guard; preserve and quarantine generated bytecode.

- 2026-10-11T08:16:46+00:00: Recorded command exit 101; command argv SHA-256
  0e8b8ec438821ebf3b5b185af8adff8c05c54025dba46010f1057a05ba1d6d2b.

- 2026-10-11T08:17:18+00:00: Recorded command exit 0; command argv SHA-256
  0e8b8ec438821ebf3b5b185af8adff8c05c54025dba46010f1057a05ba1d6d2b.

- 2026-10-11T08:18:03+00:00: Recorded command exit 0; command argv SHA-256
  82b893c0185fef6d76635523b4de3b7a2baadcdf82314f52f1db386a60d2503d.

- 2026-10-11T08:18:38+00:00: Recorded command exit 0; command argv SHA-256
  eadb1624311d49cd479c401aed6471f8f6a4eeac5e00ffdf4dc1da70c5b0b256.

- 2026-10-11T08:19:21+00:00: Recorded command exit 0; command argv SHA-256
  c7323847342774673319b7ffd857e626b77892358d868ecb570bff056d311898.

- 2026-10-11T08:20:25+00:00: Recorded command exit 0; command argv SHA-256
  2bd70e0c26aa986612d3eee65d9d3a6c101e21a93620f69f0b2353ef488adf00.

- 2026-10-11T08:21:15+00:00: Recorded command exit 0; command argv SHA-256
  c50160c33f34ec7955196138b6f40b52c9c3006f8928e00c37e1ca66932c1fee.

- 2026-10-11T08:21:51+00:00: Recorded command exit 101; command argv SHA-256
  770ef66e05618edaf585abd6d89f8f4b9cea6dae022116745e9147ef57d01a1d.

- 2026-10-11T08:23:11+00:00: Complete asb-cli suite exposed three concrete failures: optional
  perf/bpftool host-tool remediation was incorrectly asserted as ASB-installable (test fixed to
  require install syntax only for mandatory tools); public command inventory lacked its preflight
  golden fixture (fixture row added); one existing human error-presentation assertion was
  environment/baseline-sensitive and showed output-parent wording, to be rechecked independently.

- 2026-10-11T08:23:33+00:00: Recorded command exit 1; command argv SHA-256
  6e451b47efa59de3b300c2fe3ff0278cab5c6d099c43967c15fb3b77515626c2.

- 2026-10-11T08:23:58+00:00: Recorded command exit 0; command argv SHA-256
  82b893c0185fef6d76635523b4de3b7a2baadcdf82314f52f1db386a60d2503d.

- 2026-10-11T08:24:47+00:00: Recorded command exit 101; command argv SHA-256
  634ca9b663d7d070b704cdbbf6ef7bfaccf7ae3bff834259c9289b273eaac204.

- 2026-10-11T08:25:36+00:00: Recorded command exit 0; command argv SHA-256
  634ca9b663d7d070b704cdbbf6ef7bfaccf7ae3bff834259c9289b273eaac204.

- 2026-10-11T08:26:21+00:00: Recorded command exit 0; command argv SHA-256
  71438883e0e7f3c41f793a3d738713da6206360a4d1273aa78f4b4aaa64205e4.

- 2026-10-11T08:27:10+00:00: Recorded command exit 0; command argv SHA-256
  fb41037a684c4eacabd62eb60a0ae25a18a6247d6c01ca1a8338a3de1e54b7d4.

- 2026-10-11T08:27:39+00:00: Recorded command exit 101; command argv SHA-256
  770ef66e05618edaf585abd6d89f8f4b9cea6dae022116745e9147ef57d01a1d.

- 2026-10-11T08:27:57+00:00: Full asb-cli rerun is deterministic for AR-1784 changes: 348/349
  passed; only existing
  human::tests::error_presentations_name_cause_state_and_recovery_without_private_text fails on
  legacy output-parent-unavailable wording (expected state-change phrase absent; output says
  requested input/configuration needs correction and no successful state change). New
  dependency_registry tests all pass, and preflight public-command golden/inventory test passes. Do
  not mask or weaken unrelated diagnostic gate.

- 2026-10-11T08:29:01+00:00: Recorded command exit 0; command argv SHA-256
  c7323847342774673319b7ffd857e626b77892358d868ecb570bff056d311898.
