---
{
  "branch": "feature/ar-1824-cli2key-public-composition",
  "checkpoint_commit": "",
  "claim_expires": "",
  "depends_on": [],
  "id": "AR-1824",
  "next_action": "Implement the cli2key provider-plan/catalog and direct runtime-authority composition seam without reopening AR-1797.",
  "observed_branch": "feature/ar-1824-cli2key-public-composition",
  "observed_dirty": 0,
  "observed_head": "e91116de10ebd306a95b8e744f2e4dbfbd0688fb",
  "owner": "",
  "plan": "../plans/AR-1824-cli2key-public-composition.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "sha256:4953306bb1bb076302ad15a56d1e2ec37b3b1a9e51a025593124ef2eb14d7a66",
    "evidence_ref": "quality/AR-1824-cli2key-public-composition-postmerge-receipt.json",
    "spec_ref": "specs/AR-1824.json",
    "spec_revision": 1,
    "status": "pass"
  },
  "spec_ref": "specs/AR-1824.json",
  "spec_revision": 1,
  "status": "done",
  "summary": "Add public cli2key provider-plan/catalog construction and direct runtime-authority composition.",
  "task_revision": 31,
  "title": "cli2key public provider composition",
  "updated_at": "2026-10-12T16:45:17+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1824-cli2key-public-composition"
}
---

AR-1797's signed runtime sidecar API and lifecycle tests exist, but its blocked review found that the normal ASB CLI cannot construct a cli2key provider plan from an installed project-local receipt and cannot compose the runtime-owned authority into direct run or sweep dispatch. Implement only that missing product seam. Preserve the existing no-secret, loopback-only, no-fallback, runtime-owned authority boundary; do not reopen or rewrite AR-1797 evidence. Development fake qualification is required; an opt-in live probe may run locally but must never be a CI or production prerequisite.

- 2026-10-12T16:00:00+00:00: Created as a successor repair for blocked AR-1797; original evidence remains immutable.

- 2026-10-12T15:52:55+00:00: Dependencies are intentionally empty: this successor repairs the
  independent public composition seam while preserving blocked AR-1797.

- 2026-10-12T15:52:55+00:00: Claimed by ar1772-provenance-repair.

- 2026-10-12T15:53:27+00:00: Recorded command exit 0; command argv SHA-256
  a852ecbeb283b9ce27c7a89ba93b2e4a01e9d30a5f77e8619a19b5c872e9773d.

- 2026-10-12T15:54:41+00:00: Recorded command exit 1; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-12T15:55:02+00:00: Recorded command exit 1; command argv SHA-256
  290a48b240a9df90cd11432fb7ac3c3d2b92d1c9f9d09861f7078778a158cf6c.

- 2026-10-12T15:55:10+00:00: Recorded command exit 0; command argv SHA-256
  99449628356f508a1e1be348d4abf60e56c88da213bcdf417293f1d945060d01.

- 2026-10-12T15:55:40+00:00: Recorded command exit 0; command argv SHA-256
  bcb7265cbad1b4739c25021b458a000d6d7c56ab62143a804657a4904d02a623.

- 2026-10-12T15:55:58+00:00: Heartbeat by ar1772-provenance-repair.

- 2026-10-12T15:56:10+00:00: Recorded command exit 0; command argv SHA-256
  51884702f8e841c30f87e039e6c687c6a40213aeff51d0766d16bb749fab0c62.

- 2026-10-12T15:56:16+00:00: Recorded command exit 0; command argv SHA-256
  7e7c086ad8e6c72b22d1f8d92ddfa5bcdbfc4d0f1a241db17f369b39276d0eb9.

- 2026-10-12T15:56:57+00:00: Recorded command exit 0; command argv SHA-256
  129297cb6c18a6de0dad66d58b54be92caac0daa148048cdb0d67bd3ffb4b5a2.

- 2026-10-12T15:57:17+00:00: Recorded command exit 0; command argv SHA-256
  8919395f9ba589e98b2fddb9400f4e4491e698365e8268ede6baafbfdbc46f0d.

- 2026-10-12T15:57:37+00:00: Heartbeat by ar1772-provenance-repair.

- 2026-10-12T15:58:20+00:00: Heartbeat by ar1772-provenance-repair.

- 2026-10-12T15:58:24+00:00: Recorded command exit 0; command argv SHA-256
  06690f533b4568cf3e5a9bb55dfd6adc28d1c464f97fdc948f560d882340fb68.

- 2026-10-12T15:58:31+00:00: Recorded command exit 101; command argv SHA-256
  06254e25f0584b220e984bec230a5673b10d587c574a26184ff12d40cdfd9e33.

- 2026-10-12T15:58:33+00:00: Recorded command exit 0; command argv SHA-256
  05662bf361f11a3ba5c5a5b581ea52be713b97d5f5b6145bf07af4515815c06e.

- 2026-10-12T15:59:28+00:00: Recorded command exit 0; command argv SHA-256
  d9e31fcca60b1ee42fe3b9c6747718a80346282554fd619f4645a6ba3485c0ed.

- 2026-10-12T16:02:30+00:00: Heartbeat by ar1772-provenance-repair.

- 2026-10-12T16:02:30+00:00: Recorded command exit 0; command argv SHA-256
  01be15aabaacf9fb5de958279ddbc6478f5aac9e8cbaef8177458bc9ab0bad9a.

- 2026-10-12T16:26:18+00:00: Heartbeat by ar1772-provenance-repair.

- 2026-10-12T16:32:02+00:00: Heartbeat by ar1772-provenance-repair.

- 2026-10-12T16:44:15+00:00: Recorded command exit 0; command argv SHA-256
  88b62a8a060cab34af5c76383ce523dba76d2f7806894cc4a82710ae36858a50.

- 2026-10-12T16:44:49+00:00: Recorded command exit 0; command argv SHA-256
  88b62a8a060cab34af5c76383ce523dba76d2f7806894cc4a82710ae36858a50.

- 2026-10-12T16:44:58+00:00: Recorded command exit 0; command argv SHA-256
  01c491b3ffbebbda29bac013d913a6a346ec22e9c6d0a5f1f95885a4a5516003.

- 2026-10-12T16:45:12+00:00: PR #616 exact head c2745e04 passed all checks; signed recovery PR #617
  exact head 448c35dc passed all checks; local merge_pr integration e058ea2b has exact parents/tree,
  SSH signature, DCO, and all 13 exact-main required workflows passed. AR-1797 remains blocked; no
  TLC development dependency.

- 2026-10-12T16:45:17+00:00: Accepted exact signed protected-main integration e058ea2b after PR #616
  web-merge provenance repair via PR #617; all exact-main workflows green. Preserve AR-1797 blocked
  record and no TLC development dependency.
