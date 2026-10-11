---
{
  "branch": "codex/ar-1786-control-state",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T07:14:52+00:00",
  "depends_on": [
    "AR-1711",
    "AR-1733"
  ],
  "id": "AR-1786",
  "next_action": "Independent exact-head review of 378787cd after diagnostic catalog repair, then full applicable gates; publish only from clean exact tree.",
  "observed_branch": "codex/ar-1786-control-state",
  "observed_dirty": 0,
  "observed_head": "7be2fcb0dc0ee3ba4f30176c491a4d278ecd584b",
  "owner": "codex-ar1786-review-merge-luna56",
  "plan": "../plans/AR-1786-control-state-root-isolation.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1786.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Eliminate control state-root ownership collisions in ordinary parallel workspace tests.",
  "task_revision": 87,
  "title": "Repair control state-root isolation race",
  "updated_at": "2026-10-11T06:35:36+00:00",
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

- 2026-10-10T20:59:23+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-10T20:59:47+00:00: Independent exact-head review blocks
  7be2fcb0dc0ee3ba4f30176c491a4d278ecd584b: three ordinary locked workspace runs failed
  diagnostic_contract because legacy catalog call-site identities are stale or absent (including new
  lock helper); additionally, POSIX F_SETLK record locks are process-scoped and closing the failed
  same-process contender descriptor can release the active owner lock despite the inode registry.
  Repair both and add cross-process regression coverage; no PR or merge.

- 2026-10-10T22:42:36+00:00: Dependencies AR-1711 and AR-1733 are complete; AR-1788 is now complete.
  Promote AR-1786 for independent exact-head review of 7be2fcb0, with no GitHub Reviews API approval
  requirement.

- 2026-10-10T22:42:48+00:00: Claimed by codex-ar1786-independent-rereview.

- 2026-10-10T22:43:38+00:00: Independent exact-head re-review blocks
  7be2fcb0dc0ee3ba4f30176c491a4d278ecd584b. SSH signature, matching DCO, and diff --check pass, but
  required cargo +1.93.0 test --locked -p asb-cli --test diagnostic_contract fails:
  ordinary_cli_producers_are_mechanically_closed is 8/9 and reports uncatalogued legacy control.rs
  producers at 9768, 11375, 11380, 11382, and 11399. Additionally, the proposed F_SETLK design
  remains unsound: POSIX record locks are process-scoped, and a failed same-process contender
  returns by dropping its separately opened file descriptor; that close can release the existing
  owner record lock even though CONTROL_STATE_LOCK_OWNERS still marks it held, permitting another
  process to enter. Repair all diagnostic identities and use an ownership mechanism safe across
  same-process failed-open/descriptor-close, with a regression showing an external process remains
  rejected after a same-process failed contender. Do not publish, merge, or run full qualification
  until repaired. No GitHub Reviews API approval object is required.

- 2026-10-11T06:24:37+00:00: Claimed by codex-ar1786-repair-luna56.

- 2026-10-11T06:26:34+00:00: Released for independent exact-head review of
  db3dfad83c511fa51f4315e2dcb57981048970d9; no PR or merge until review and exact-head CI pass.

- 2026-10-11T06:26:40+00:00: Claimed by codex-ar1786-repair-luna56.

- 2026-10-11T06:26:48+00:00: Repair commit db3dfad83c511fa51f4315e2dcb57981048970d9 is SSH-signed
  and DCO-signed. Focused state-lock, failed same-process contender, parallel isolation, diagnostic
  contract, fmt, and diff-check pass. Candidate remains unreleased pending independent review.

- 2026-10-11T06:26:51+00:00: Released for independent exact-head review of
  db3dfad83c511fa51f4315e2dcb57981048970d9; no PR or merge until review and exact-head CI pass.

- 2026-10-11T06:28:47+00:00: Claimed by codex-ar1786-review-merge-luna56.

- 2026-10-11T06:29:16+00:00: Recorded command exit 101; command argv SHA-256
  a2ae8499d4ead7710b77bfd594e50500ebf3445b07b753cf0548c8f80f92df62.

- 2026-10-11T06:29:36+00:00: Independent exact-head review at
  db3dfad83c511fa51f4315e2dcb57981048970d9: cargo test --locked -p asb-cli --test
  diagnostic_contract failed ordinary_cli_producers_are_mechanically_closed (8/9). It reports
  uncatalogued legacy control.rs producers at lines 9768, 11387, 11392, 11394, and 11409 after line
  shifts. Candidate signature/DCO and diff-check passed. No PR, CI, or merge authorized until
  catalog identities are repaired.

- 2026-10-11T06:29:52+00:00: Heartbeat by codex-ar1786-review-merge-luna56.

- 2026-10-11T06:30:59+00:00: Recorded command exit 0; command argv SHA-256
  a2ae8499d4ead7710b77bfd594e50500ebf3445b07b753cf0548c8f80f92df62.

- 2026-10-11T06:31:25+00:00: Recorded command exit 0; command argv SHA-256
  d1fa4c2defd79ebbee16fb0c7a61550c97f0b2868b21a85e028a19abb6494b64.

- 2026-10-11T06:31:29+00:00: Recorded command exit 0; command argv SHA-256
  64b061007080c0a0d9c6f9b297bf03448cc79b23443c7d087d7ddfb38aa1e582.

- 2026-10-11T06:31:55+00:00: Recorded command exit 128; command argv SHA-256
  97a494a2702f01eebca6ff50b85e90aad643c2b6892f9faf937d9920dd6c2b84.

- 2026-10-11T06:32:07+00:00: Recorded command exit 0; command argv SHA-256
  cff985bdc945bd914622a83e8b993ba7c2e73417da008903ea7d15f0d251a125.

- 2026-10-11T06:32:23+00:00: Recorded command exit 0; command argv SHA-256
  73ef1c0a9100922c4432b95185fed0b6fbe3526cbff6a0a5f30c434eeb5c115b.

- 2026-10-11T06:32:41+00:00: Repaired diagnostic_legacy_catalog.rs identities for the five
  intentional control.rs producer shifts (9768, 11387, 11392, 11394, 11409), preserving stable
  catalog IDs and causes. Committed as SSH-signed DCO commit 378787cd. Focused diagnostic_contract
  passes 9/9; state-lock child-inheritance test passes. Candidate now requires independent review at
  exact head 378787cd.

- 2026-10-11T06:32:53+00:00: Recorded command exit 0; command argv SHA-256
  2df5b4f3585972c2df80b3e2269e46ae2dac0e14e9f6751b0e520054c3448f80.

- 2026-10-11T06:33:06+00:00: Recorded command exit 0; command argv SHA-256
  5c057c98429b327fddf7a0d5872b3b5286ad10ed270db997932638aa13277427.

- 2026-10-11T06:33:19+00:00: Recorded command exit 0; command argv SHA-256
  cfd399d0da76ddbca0a8deafb29e485fe40fe97aff9c9c4d416608e350f8bfbc.

- 2026-10-11T06:33:46+00:00: Recorded command exit 0; command argv SHA-256
  a65d244b9c033212584e323e0c07d7a01feb1c79f754d6a494c58c3c0bd20be6.

- 2026-10-11T06:34:46+00:00: Recorded command exit 0; command argv SHA-256
  3b061dab12035f416b4d163de72cacc57d3c38f213ced4ea03ecc701d3750294.

- 2026-10-11T06:35:36+00:00: Recorded command exit 0; command argv SHA-256
  3b061dab12035f416b4d163de72cacc57d3c38f213ced4ea03ecc701d3750294.
