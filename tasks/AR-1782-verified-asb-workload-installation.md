---
{
  "branch": "feature/ar-1782-verified-asb-workload-installation",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T16:00:48+00:00",
  "depends_on": [
    "AR-1781"
  ],
  "id": "AR-1782",
  "next_action": "Implement asb workload install/list/status/select/repair/update/remove for every development-supported official external workload bundle.",
  "observed_branch": "feature/ar-1782-verified-asb-workload-installation",
  "observed_dirty": 0,
  "observed_head": "a1090297dede4412bcf1487d2d7bc00315c69dc5",
  "owner": "ar1782_workload_install_luna56",
  "plan": "../plans/AR-1782-verified-asb-workload-installation.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1782.json",
    "spec_revision": 2,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1782.json",
  "spec_revision": 2,
  "status": "in_progress",
  "summary": "Implement dedicated verified remote external workload installation and lifecycle through asb workload.",
  "task_revision": 12,
  "title": "Verified asb workload installation",
  "updated_at": "2026-10-11T14:05:54+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1782-verified-asb-workload-installation"
}
---

Add `asb workload install <supported-id> [--project PATH]` and `list`, `status`,
`select`, `repair`, `update`, and `remove` lifecycle commands. Install resolves only the
AR-1781 catalog, downloads the official primary-source workload release into a
project-local workload root, verifies immutable identity/proof/license/size and
archive boundaries, performs only declared bounded preparation, validates the
adapter-ready structure, and atomically records a workload receipt.

The command must support all AR-1781 development workload IDs, including
SWE-mini, not merely list them. It must preserve a good prior workload on failed update,
never execute downloaded installer scripts, require root, or mutate global package
manager state. Read-only operations are network-free. Download, integrity,
license, preparation, storage, incompatibility, stale/revoked, and offline cases
must remain separate actionable outcomes. Development operation must not be
blocked by production authorization or hosted-release verification requirements;
the pinned primary-source identity, declared constraints, and bounded preparation
remain mandatory.

- 2026-10-11T13:53:25+00:00: Promote dependency-safe AR-1782 now that AR-1781 is durably done with
  exact-main evidence.

- 2026-10-11T13:53:59+00:00: Claimed by ar1782_workload_install_luna56.

- 2026-10-11T13:54:05+00:00: Recorded command exit 0; command argv SHA-256
  ff6d1fa7a7185010f75fcfafe4bdc1bdd5b9804d8c2b3e7abdd8a1b96cd81c63.

- 2026-10-11T14:00:48+00:00: Heartbeat by ar1782_workload_install_luna56.

- 2026-10-11T14:04:41+00:00: Recorded command exit 0; command argv SHA-256
  e274074a17e0682b5edf79b5caca577a41520ed9ca8daf9342f0433b678582a1.

- 2026-10-11T14:05:07+00:00: Recorded command exit 0; command argv SHA-256
  acc9cf8127a84779f740f6350aa3a1616ba38c8dc1c9797006e2886289fc16e0.

- 2026-10-11T14:05:20+00:00: Recorded command exit 0; command argv SHA-256
  e6e449e9d1f470db7ec6f673f670b5e31f0066cbbd41fd7b2de98c8bdbed15c6.

- 2026-10-11T14:05:31+00:00: Recorded command exit 0; command argv SHA-256
  afb0c01c2d49638af443d2502727e1b5ac6bfd221160d69ddb93f495dd751142.

- 2026-10-11T14:05:54+00:00: Recorded command exit 8; command argv SHA-256
  78f325bde3962aea10f3844ad0018af88b2cf3e001f1e02858b67193c031b6ae.
