---
{
  "branch": "repair/ar-1834-ar1775-protected-main-identity",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-13T01:42:49+00:00",
  "depends_on": [],
  "id": "AR-1834",
  "next_action": "Independent exact-head review approved 4107fcd177efa675f74e1ae08d0ee0bfa15c5216; all 14 PR checks are terminal-success. Merge only through tools/integration/merge_pr.py; do not web-merge.",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "ar1805_postmerge_repair",
  "plan": "../plans/AR-1834-ar1775-protected-main-identity-recovery.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "hosted",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1834.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1834.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Recover AR-1775 protected-main provenance with the established local signed integration identity without rewriting historical merge 356a669e",
  "task_revision": 39,
  "title": "AR-1775 protected-main local identity recovery",
  "updated_at": "2026-10-12T22:46:10+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1834-ar1775-protected-main-identity"
}
---

Historical AR-1775 merge `356a669e0c6ebbb6cd3675a119cba16fea486283` is immutable evidence. Its two-parent topology, reviewed tree, SSH signature and DCO are preserved, but GitHub Web Flow committed the protected merge as `GitHub <noreply@github.com>`. Portable protected-main provenance, Repository quality and Rust DCO therefore correctly reject it against the exact local integration identity.

This successor is metadata-only and forward-only. It must not alter `LOCAL_COMMITTER`, accept arbitrary signers, rewrite main, weaken any gate, change AR-1779 or AR-1783, or touch AR-1826--AR-1832. Use the established AR-1794/AR-1820 recovery path: clean isolated worktree, signed-DCO recovery topic preserving the current tree, independent exact-head review, focused/full gates, and `tools/integration/merge_pr.py` local signed merge.

- 2026-10-13T00:30:00+00:00: Created from current exact-main failures for AR-1775; historical merge and all failed workflow evidence remain immutable inputs.

- 2026-10-12T22:27:30+00:00: Dependencies empty; promote narrow AR-1775 protected-main identity
  recovery preserving historical merge 356a669e and excluding AR-1779/1783 and AR-1826-1832

- 2026-10-12T22:27:39+00:00: Claimed by ar1805_postmerge_repair.

- 2026-10-12T22:27:46+00:00: Recorded command exit 0; command argv SHA-256
  aae66aab4fdf8f873e2ba3b94e322d63ce9d5491cf6dfef4475319458372bc01.

- 2026-10-12T22:27:54+00:00: Recorded command exit 0; command argv SHA-256
  80cb08bc1f85ed49bda6f8d6be0029c68080518b66963177023730e379c8aef5.

- 2026-10-12T22:28:06+00:00: Recorded command exit 0; command argv SHA-256
  f3eeefa5152e872cfea64640e3888a4b062d87e057c8459547f2601c98bfc6a9.

- 2026-10-12T22:28:17+00:00: Recorded command exit 0; command argv SHA-256
  45d32099e98947b105f1ba822a5852446f68ff5291da006db92652489a8533fb.

- 2026-10-12T22:28:28+00:00: Recorded command exit 1; command argv SHA-256
  259bda4af8fb24e0ae4d4e6e32a44b723d2512ddb56ab47abd0dee304ab40b36.

- 2026-10-12T22:28:43+00:00: The initial focused repository-policy invocation failed closed because
  the recovery topic was supplied as abbreviated SHA 4107fcd1; repository_policy requires full
  lowercase 40-character commit identities. No product or state contract changed; retry with the
  exact full SHA.

- 2026-10-12T22:28:49+00:00: Recorded command exit 0; command argv SHA-256
  78b3af9ea21dfa24c5f4e6ef8075826f269ea693101d1c55db2a6800d63105f7.

- 2026-10-12T22:29:14+00:00: Recorded command exit 0; command argv SHA-256
  cc286686915075c87a0404fa93736b9508c3c7199bbb678b5e15984489777beb.

- 2026-10-12T22:30:27+00:00: Recorded command exit 0; command argv SHA-256
  c91971cb9346daffdb348fc5a435e52a22fc06f3718d0d7c9b948e9a91caaf5b.

- 2026-10-12T22:31:03+00:00: Recorded command exit 0; command argv SHA-256
  2ec31af133e16f6db69c8772912b69ce47d197e6db1e55b2b13cda706e406580.

- 2026-10-12T22:31:16+00:00: Recorded command exit 0; command argv SHA-256
  267c41b023d0a59335bfa7f68cbcea49527657503f189ad425695e4a85dd9561.

- 2026-10-12T22:31:30+00:00: Recorded command exit 8; command argv SHA-256
  f481b1f12e540160877c01fb3534a947cf2c523f558d8bdb273eadf7cc435f35.

- 2026-10-12T22:32:42+00:00: Recorded command timeout; classification=SUBPROCESS_TIMEOUT;
  deadline=60.0s; command argv SHA-256
  096b0655dbac80be4b47a92ee0808f45b7af4e16a148353b99139ee4b23b36dd.

- 2026-10-12T22:32:51+00:00: Recovery branch repair/ar-1834-ar1775-protected-main-identity is pushed
  at exact head 4107fcd177efa675f74e1ae08d0ee0bfa15c5216; PR #626 is open. The gh checks --watch
  command exited 8 because the bounded 60-second monitoring window elapsed while required checks
  remained pending; this is a monitoring timeout, not a check failure. Current observed checks:
  Portable provenance, headers, and AWQ shadow pass; remaining required checks pending.

- 2026-10-12T22:32:57+00:00: Recorded command exit 8; command argv SHA-256
  f481b1f12e540160877c01fb3534a947cf2c523f558d8bdb273eadf7cc435f35.

- 2026-10-12T22:33:45+00:00: Recorded command exit 8; command argv SHA-256
  f481b1f12e540160877c01fb3534a947cf2c523f558d8bdb273eadf7cc435f35.

- 2026-10-12T22:34:11+00:00: The subsequent bounded gh pr checks poll exited 8 because GitHub
  reports pending checks; this is the gh pending-status exit code, not a workflow failure. No
  required check has failed. Current terminal passes remain Portable provenance, headers, AWQ
  shadow, Kani, Loom and retained faults; other required checks are still pending.

- 2026-10-12T22:34:16+00:00: Recorded command exit 8; command argv SHA-256
  f481b1f12e540160877c01fb3534a947cf2c523f558d8bdb273eadf7cc435f35.

- 2026-10-12T22:34:54+00:00: Recorded command exit 8; command argv SHA-256
  f481b1f12e540160877c01fb3534a947cf2c523f558d8bdb273eadf7cc435f35.

- 2026-10-12T22:35:03+00:00: Recorded command exit 0; command argv SHA-256
  b45a658eee3a6a0dc2dc5e03c6b71f2d22df4f3b7222eae2758c25b98dd17428.

- 2026-10-12T22:35:59+00:00: Recorded command exit 8; command argv SHA-256
  f481b1f12e540160877c01fb3534a947cf2c523f558d8bdb273eadf7cc435f35.

- 2026-10-12T22:36:17+00:00: Recorded command exit 0; command argv SHA-256
  5eeeebb98e8e1f06568b90203e8b05b4ec6f1360a5995b8a18b7b298ff7779f2.

- 2026-10-12T22:36:27+00:00: PR #626 exact head 4107fcd177efa675f74e1ae08d0ee0bfa15c5216 currently
  has 11 terminal-success hosted checks. Remaining runs are Repository quality 38317132551, Rust
  verification 38317132596, and emulated AArch64 portability 38317132577, all in progress; no hosted
  failures observed.

- 2026-10-12T22:37:23+00:00: Recorded command exit 0; command argv SHA-256
  5eeeebb98e8e1f06568b90203e8b05b4ec6f1360a5995b8a18b7b298ff7779f2.

- 2026-10-12T22:37:58+00:00: Recorded command exit 8; command argv SHA-256
  f481b1f12e540160877c01fb3534a947cf2c523f558d8bdb273eadf7cc435f35.

- 2026-10-12T22:38:11+00:00: Heartbeat by ar1805_postmerge_repair.

- 2026-10-12T22:39:07+00:00: Recorded command exit 8; command argv SHA-256
  f481b1f12e540160877c01fb3534a947cf2c523f558d8bdb273eadf7cc435f35.

- 2026-10-12T22:41:05+00:00: Recorded command exit 8; command argv SHA-256
  f481b1f12e540160877c01fb3534a947cf2c523f558d8bdb273eadf7cc435f35.

- 2026-10-12T22:42:15+00:00: Recorded command exit 8; command argv SHA-256
  f481b1f12e540160877c01fb3534a947cf2c523f558d8bdb273eadf7cc435f35.

- 2026-10-12T22:42:49+00:00: Heartbeat by ar1805_postmerge_repair.

- 2026-10-12T22:43:50+00:00: Recorded command exit 0; command argv SHA-256
  f481b1f12e540160877c01fb3534a947cf2c523f558d8bdb273eadf7cc435f35.

- 2026-10-12T22:44:15+00:00: Independent Luna review receipt: exact PR #626 head
  4107fcd177efa675f74e1ae08d0ee0bfa15c5216 is one signed-DCO metadata-only child of historical
  356a669e0c6ebbb6cd3675a119cba16fea486283; candidate tree equals historical tree and diff is empty.
  Candidate SSH signature verifies as Martin Beck <martin.beck2@gmx.de>; repository policy and
  portable provenance pass; privacy, signer identity, arbitrary-signer rejection, historical
  preservation, and unrelated-scope checks pass. Focused/full local gates pass: fmt, clippy,
  workspace tests, rustdoc, release build. PR #626 exact-head hosted checks all 14 terminal-success
  (including Policy/coverage/supply chain, Rust, emulated AArch64, provenance, AWQ, TUI, Kani, Loom,
  fuzz, retained faults, platform, credential-free, and headers). Approved for signed local merge
  only; no merge performed.

- 2026-10-12T22:44:44+00:00: Recorded command exit 1; command argv SHA-256
  97aa923a11b24b3ee346e0399a0f4f0bdeb2be5266ed6a878d78d4cbfa4ba806.

- 2026-10-12T22:45:11+00:00: Recorded command exit 0; command argv SHA-256
  a3be9d6a727177db9dda504114afedaf4e1684bccdf7c46fdc3f43655b2f528d.

- 2026-10-12T22:45:53+00:00: Recorded command exit 0; command argv SHA-256
  8df2de23b021e41eb9310906ac11af7d07bf7cacf934596b8e3e22d4722f5bad.

- 2026-10-12T22:46:10+00:00: Recorded command exit 0; command argv SHA-256
  7bffcf86d3ba2afc123ef17bd40405575ade3286612405fb5a110863092ccc68.
