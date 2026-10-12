---
{
  "branch": "",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-12T23:54:29+00:00",
  "depends_on": [
    "AR-1689"
  ],
  "id": "AR-1833",
  "next_action": "Promote and claim after AR-1689 receipt repair is authorized.",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "ar1689-tui-qualification-luna",
  "plan": "../plans/AR-1833-repair-ar1689-receipt.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "journey",
    "evidence_digest": "sha256:7eab9f858b87e08bcc4849e6711298c144f77e5e60838fde5b1bd54c740d2928",
    "evidence_ref": "quality/AR-1689-paired-tui-fresh-user-20261012.json",
    "spec_ref": "specs/AR-1833.json",
    "spec_revision": 1,
    "status": "pass"
  },
  "spec_ref": "specs/AR-1833.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Repair malformed AR-1689 paired qualification evidence before relying on its done state.",
  "task_revision": 7,
  "title": "Repair AR-1689 receipt bytes",
  "updated_at": "2026-10-12T21:55:50+00:00",
  "worktree_key": ""
}
---

Repair AR-1689’s malformed receipt bytes without changing the reviewed paired qualification content.

- 2026-10-12T21:54:18+00:00: Promote P0 evidence repair because AR-1689 receipt bytes were malformed
  after release.

- 2026-10-12T21:54:29+00:00: Claimed by ar1689-tui-qualification-luna.

- 2026-10-12T21:54:36+00:00: Recorded command exit 0; command argv SHA-256
  9410754323bf30b1f3133ec96282c7505ead686f1672ce12b5976ba115f8eb51.

- 2026-10-12T21:55:01+00:00: Recorded command exit 0; command argv SHA-256
  5f9a636064860b1d2ac68df15f22c2b99cb69058207b1b1832e9d33110a484f1.

- 2026-10-12T21:55:25+00:00: Recorded command exit 0; command argv SHA-256
  5d35708dfd2d74ec27c9267eea0a99c0cb6e17bc6a84f84e59196622809b5251.

- 2026-10-12T21:55:50+00:00: Receipt rewritten through the AR-1833 wrapper from canonical JSON,
  validates with python json.tool, and ends in a real LF newline. Repair commit 90d65d8ba1 is
  SSH-signed and DCO-signed.
