---
{
  "branch": "codex/ar-1786-control-state",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-10T21:57:24+00:00",
  "depends_on": [
    "AR-1711",
    "AR-1733"
  ],
  "id": "AR-1786",
  "next_action": "Independent exact-head review of 7be2fcb0dc0ee3ba4f30176c491a4d278ecd584b, then exact-head CI; no PR or merge before both pass.",
  "observed_branch": "codex/ar-1786-control-state",
  "observed_dirty": 0,
  "observed_head": "7be2fcb0dc0ee3ba4f30176c491a4d278ecd584b",
  "owner": "codex-ar1786-independent-review",
  "plan": "../plans/AR-1786-control-state-root-isolation.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1786.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Eliminate control state-root ownership collisions in ordinary parallel workspace tests.",
  "task_revision": 60,
  "title": "Repair control state-root isolation race",
  "updated_at": "2026-10-10T20:58:42+00:00",
  "worktree_key": "agent-systems-benchmark-ar1786-control-product"
}
---

This development repair is independent of GitHub Reviews API approvals. It requires
independent technical review, exact-head CI, signed DCO integration, and post-merge
verification. It must not weaken control ownership or convert the full workspace
gate into a serial-only or exclusion-based check.

- 2026-10-10T20:35:00+00:00: Created from the AR-1785 independent review blocker. Two
  ordinary `cargo test --locked --workspace` runs at e9ad590e failed in distinct
  `asb-cli::control` tests with `control state root is already owned`, while isolated
  runs passed. No AR-1785 publication or merge is authorized until this race is repaired.

- 2026-10-10T20:34:12+00:00: AR-1711 and AR-1733 are done; promote control state-root isolation
  repair discovered by AR-1785 exact-head review

- 2026-10-10T20:36:22+00:00: Claimed by codex-ar1786-control-race.

- 2026-10-10T20:37:03+00:00: Heartbeat by codex-ar1786-control-race.

- 2026-10-10T20:37:06+00:00: Recorded command exit 0; command argv SHA-256
  599225c1489a39ad2dc00bec49906045b80e7ec1d6d2e85a7979fcc17839ea18.

- 2026-10-10T20:37:34+00:00: Heartbeat by codex-ar1786-control-race.

- 2026-10-10T20:37:44+00:00: Recorded command exit 0; command argv SHA-256
  38bea93da73d24de012ae726701344927ad83be2b5f706d5a230e5ad5115ca3e.

- 2026-10-10T20:37:51+00:00: Recorded command exit 128; command argv SHA-256
  395df38e2a83dc2b6b853dd3add8fe4b6f5d7fca1de922c0a2c28c95a319196f.

- 2026-10-10T20:38:00+00:00: Recorded command exit 0; command argv SHA-256
  bd14aa25f0c6c589a618820a4804aca4bd35a25ac085f282cc64454a9a19b5d0.

- 2026-10-10T20:38:07+00:00: Recorded command exit 0; command argv SHA-256
  baa0f415515a673757c523a19f778c74680658da9724e8f0ba1a415a4f4e58fc.

- 2026-10-10T20:38:16+00:00: Recorded command exit 0; command argv SHA-256
  190aac4dfe591738746fa13f4f66c4344a9eccc776d7256b78108bbb8817487a.

- 2026-10-10T20:38:23+00:00: Recorded command exit 0; command argv SHA-256
  89901aa84043e8dc7e1eb76ddfef2f9c56172ab8c80f666ef8217add6a12066d.

- 2026-10-10T20:38:36+00:00: Heartbeat by codex-ar1786-control-race.

- 2026-10-10T20:38:44+00:00: Recorded command exit 0; command argv SHA-256
  5b6230b60399d77c6694e286d8491f38576329dbdee89134307620337415f5d2.

- 2026-10-10T20:38:53+00:00: Recorded command exit 255; command argv SHA-256
  af696a373700417fb4a7257f075a6d698c4dad9bb06efae43cb82827e6ac5d80.

- 2026-10-10T20:39:03+00:00: Recorded command exit 0; command argv SHA-256
  70cdbd73cd762f78a089f3379da0900b7b222c8632586bec24abcd7345ca72ec.

- 2026-10-10T20:39:33+00:00: Recorded command exit 0; command argv SHA-256
  d04aa803a9fcb854b247572f9b417fd1935a57ca978bb887d2e7c188e99db043.

- 2026-10-10T20:39:41+00:00: Recorded command exit 0; command argv SHA-256
  b04e51d6a8a1cd1d991d96d1b0eeb7f5236f1056ccb95c8f404b26c181dc4797.

- 2026-10-10T20:40:55+00:00: Recorded command exit 0; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:41:09+00:00: Heartbeat by codex-ar1786-control-race.

- 2026-10-10T20:41:36+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:42:11+00:00: Ordinary locked workspace rerun reproduced control state-root ownership
  failure in recording-campaign restart; baseline prior rerun passed. Focused source audit in
  progress; no test serialization or exclusions.

- 2026-10-10T20:42:19+00:00: Recorded command exit 0; command argv SHA-256
  c38d9c9ba7c2c51cf76919f501ff285f748692b9e046dc5a87f24a4903cb35f8.

- 2026-10-10T20:42:51+00:00: Recorded command exit 0; command argv SHA-256
  b878e982f06a06f2dc51e81453d94d62942ddc916c05dd06b834c0b2bc45a2ef.

- 2026-10-10T20:43:00+00:00: Recorded command exit 0; command argv SHA-256
  ebfe524ae780b8318016ce20f74b9dcb5f2f576a1e118465bf07f36b64a21cd6.

- 2026-10-10T20:43:48+00:00: Recorded command exit 0; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:44:49+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:45:28+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:46:21+00:00: Heartbeat by codex-ar1786-control-race.

- 2026-10-10T20:46:59+00:00: Recorded command exit 0; command argv SHA-256
  3d4cc54827fa0ae5c402321266e4a8e9a84a69a17f7ff27b7a0f0c858d454784.

- 2026-10-10T20:47:53+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:48:12+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:49:18+00:00: Recorded command exit 0; command argv SHA-256
  b878e982f06a06f2dc51e81453d94d62942ddc916c05dd06b834c0b2bc45a2ef.

- 2026-10-10T20:49:36+00:00: Recorded command exit 0; command argv SHA-256
  61c20235acf6e8e448b1670c7eec5ea2a1daef5d2ed91423522cd6185fbb5e3b.

- 2026-10-10T20:49:46+00:00: Recorded command exit 0; command argv SHA-256
  ebfe524ae780b8318016ce20f74b9dcb5f2f576a1e118465bf07f36b64a21cd6.

- 2026-10-10T20:50:16+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:51:24+00:00: Recorded command exit 1; command argv SHA-256
  7ca4558f1330f6854210b691cb99c18e6c2f4261c4cbfc2a22ce72a274c39a73.

- 2026-10-10T20:51:36+00:00: Recorded command exit 0; command argv SHA-256
  a8ead32e951460d96f5ef53ad26f5f0381d0ad41b3580927aa87ff0757275af4.

- 2026-10-10T20:51:58+00:00: Recorded command exit 101; command argv SHA-256
  7109951d44fa989265b6b6481cadfc8b40676ae39f4e04b8e1126fe99b431b01.

- 2026-10-10T20:52:14+00:00: Recorded command exit 0; command argv SHA-256
  7ca4558f1330f6854210b691cb99c18e6c2f4261c4cbfc2a22ce72a274c39a73.

- 2026-10-10T20:52:26+00:00: Recorded command exit 0; command argv SHA-256
  7109951d44fa989265b6b6481cadfc8b40676ae39f4e04b8e1126fe99b431b01.

- 2026-10-10T20:53:20+00:00: Recorded command exit 0; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:53:45+00:00: Recorded command exit 0; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:54:19+00:00: Recorded command exit 0; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:54:35+00:00: Recorded command exit 101; command argv SHA-256
  1024fc1e6cefa31b078f0b5d2ed85c7ea4586897b388bf1e308f62db58574461.

- 2026-10-10T20:55:02+00:00: Recorded command exit 0; command argv SHA-256
  1024fc1e6cefa31b078f0b5d2ed85c7ea4586897b388bf1e308f62db58574461.

- 2026-10-10T20:55:19+00:00: Recorded command exit 0; command argv SHA-256
  649e9e1ebfaf13c15c95c6360ca922c0b7c73a9a7fd2cf6943002779fca093b1.

- 2026-10-10T20:55:27+00:00: Recorded command exit 0; command argv SHA-256
  7ca4558f1330f6854210b691cb99c18e6c2f4261c4cbfc2a22ce72a274c39a73.

- 2026-10-10T20:55:51+00:00: Recorded command exit 0; command argv SHA-256
  f7f722cb81aa5b32becd345f5f931e77a4efd9b018ed504ae4528be26008099c.

- 2026-10-10T20:55:59+00:00: Recorded command exit 0; command argv SHA-256
  081d2e29d7c81d058fb9471b65995a299501fd475d64c55f8f05c360c8cd74dd.

- 2026-10-10T20:56:10+00:00: Recorded command exit 0; command argv SHA-256
  d2a2a8ad56e7e7a67cf0666e4b588fe3ad587cd1fe3ca4af0f67fac9df2bff5f.

- 2026-10-10T20:56:24+00:00: Implemented POSIX record-lock ownership plus an inode registry to
  preserve same-process exclusion while preventing child inheritance. Added child, negative, and
  16-way concurrency coverage. Commit 7be2fcb0dc0ee3ba4f30176c491a4d278ecd584b is SSH-signed and
  DCO-signed. Three ordinary locked workspace runs, fmt, clippy, rustdoc, and diagnostic contract
  pass.

- 2026-10-10T20:56:29+00:00: Released only for independent exact-head technical review of SSH-signed
  DCO commit 7be2fcb0dc0ee3ba4f30176c491a4d278ecd584b. No PR, merge, or CI conclusion is claimed.

- 2026-10-10T20:57:24+00:00: Claimed by codex-ar1786-independent-review.

- 2026-10-10T20:57:55+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:58:42+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.
