---
{
  "branch": "codex/ar-1787-lock-lifetime",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-10T23:02:30+00:00",
  "depends_on": [
    "AR-1711",
    "AR-1733"
  ],
  "id": "AR-1787",
  "next_action": "Promote and claim after dependency verification; reproduce the AR-1786 same-process lock-release flaw, repair descriptor lifetime and catalog identities, then run repeated full gates.",
  "observed_branch": "codex/ar-1787-lock-lifetime",
  "observed_dirty": 0,
  "observed_head": "b79ad1597a1cde44b0255df9d6c377614cf9cddf",
  "owner": "codex-ar1787-lock-lifetime",
  "plan": "../plans/AR-1787-control-lock-lifetime-repair.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1787.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Correct control lock descriptor lifetime and diagnostic identities after the AR-1786 review blocker.",
  "task_revision": 8,
  "title": "Repair control lock lifetime semantics",
  "updated_at": "2026-10-10T21:04:25+00:00",
  "worktree_key": "agent-systems-benchmark-ar1787-lock-lifetime"
}
---

This development repair does not require a GitHub Reviews API approval object.
Independent technical review, exact-head CI, signed DCO integration, and
post-merge verification remain mandatory. It supersedes no existing AR and must
not weaken control ownership or the ordinary workspace gate.

- 2026-10-10T21:02:00+00:00: Created from the AR-1786 independent review. Candidate
  7be2fcb0 had three ordinary workspace failures from stale diagnostic identities,
  and a P0 same-process POSIX record-lock lifetime flaw where a rejected contender's
  descriptor close could release the active owner lock. No publication or merge is authorized.

- 2026-10-10T21:01:33+00:00: AR-1711 and AR-1733 done; promote lock lifetime repair from AR-1786 P0
  review blocker

- 2026-10-10T21:02:30+00:00: Claimed by codex-ar1787-lock-lifetime.

- 2026-10-10T21:03:02+00:00: Recorded command exit 0; command argv SHA-256
  4e602e3889ec8c680e9b5df27a710c81b2e25f70a66c2525be3e11da74e0c40b.

- 2026-10-10T21:04:02+00:00: Recorded command exit 0; command argv SHA-256
  ff6d1fa7a7185010f75fcfafe4bdc1bdd5b9804d8c2b3e7abdd8a1b96cd81c63.

- 2026-10-10T21:04:09+00:00: Recorded command exit 0; command argv SHA-256
  e3c2d64cff1264cb79297f01e3210cbe622854b7e0e9236bca73ebd648a9ee3f.

- 2026-10-10T21:04:18+00:00: Recorded command exit 0; command argv SHA-256
  103c3ad2083ca670dc83e225f699608673e4c68e18bb15157fc3df4605ee53b9.

- 2026-10-10T21:04:25+00:00: Recorded command exit 0; command argv SHA-256
  312669645c274c794e787cf7fc01e58aad0657b30d423e658b5064780fa07407.
