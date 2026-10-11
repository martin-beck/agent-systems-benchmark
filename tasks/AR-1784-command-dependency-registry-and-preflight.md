---
{
  "branch": "feature/ar-1784-command-dependency-registry-and-preflight",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T10:10:16+00:00",
  "depends_on": [
    "AR-1776"
  ],
  "id": "AR-1784",
  "next_action": "Derive a complete command-to-tool/host-capability dependency registry and fail-fast preflight from all ASB command paths, adapters, monitors, and build recipes.",
  "observed_branch": "feature/ar-1784-command-dependency-registry-and-preflight",
  "observed_dirty": 0,
  "observed_head": "3031428aab47be95943b75cd753b0b5e3aeb4f6b",
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
  "task_revision": 9,
  "title": "Command dependency registry and preflight",
  "updated_at": "2026-10-11T08:12:21+00:00",
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
