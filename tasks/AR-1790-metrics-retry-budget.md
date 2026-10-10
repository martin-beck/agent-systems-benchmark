---
{
  "branch": "codex/ar-1790-metrics-retry",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-10T23:07:44+00:00",
  "depends_on": [
    "AR-1711",
    "AR-1733"
  ],
  "id": "AR-1790",
  "next_action": "Repair the effective metrics WouldBlock retry guard and add retry-budget regression coverage in the bound worktree.",
  "observed_branch": "",
  "observed_dirty": 0,
  "observed_head": "",
  "owner": "codex-ar1790-integration",
  "plan": "../plans/AR-1790-metrics-retry-budget.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_ref": "specs/AR-1790.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Repair the ineffective bounded metrics spawn retry guard found in AR-1789 review.",
  "task_revision": 91,
  "title": "Make metrics spawn retry budget effective",
  "updated_at": "2026-10-10T22:08:53+00:00",
  "worktree_key": "agent-systems-benchmark-ar1790-metrics-retry-budget"
}
---

This development repair does not require a GitHub Reviews API approval object.
Independent technical review, exact-head CI, signed DCO integration, and
post-merge verification remain mandatory.

- 2026-10-10T21:43:00Z: Created from the AR-1789 independent review. The
  candidate declares eight spawn attempts but still retries only while
  `attempt < 2`; no publication or merge is authorized until the guard and
  regression evidence are repaired.

- 2026-10-10T21:43:20+00:00: Promote retry-budget repair after AR-1789 independent review found the
  declared eight-attempt metrics retry loop still guarded by attempt < 2.

- 2026-10-10T21:43:24+00:00: Claimed by coordinator-ar1790.

- 2026-10-10T21:43:29+00:00: Recorded command exit 0; command argv SHA-256
  6ded50976c239dacd69bf4c7f27ef842a801ec99d13e89af81aaeed2d9d342ca.

- 2026-10-10T21:44:00+00:00: Recorded command exit 1; command argv SHA-256
  b6450fc1928ee39ccddd531049b97da67a0d55ecb6c887ddb58727298bb510cd.

- 2026-10-10T21:44:04+00:00: Recorded command exit 0; command argv SHA-256
  800a4baab9027da70e32cd5732f07bbe4ea594e839cc011e8b85a0231876a64a.

- 2026-10-10T21:44:27+00:00: Recorded command exit 0; command argv SHA-256
  26eb69bb1c0c6a4c4b59c48f9294513142fa9c270540a7d92b845027ac2d6000.

- 2026-10-10T21:44:34+00:00: Recorded command exit 0; command argv SHA-256
  ee675473ffc5da36d96f762baa11cb8a17a8f60e4ee9996b26d33921d6897053.

- 2026-10-10T21:44:44+00:00: Recorded command exit 0; command argv SHA-256
  b3eff22838074d693bfb44947f9c5dcd1c94b678d5bb7c4ea525dd65c3aa0d06.

- 2026-10-10T21:44:51+00:00: Recorded command exit 0; command argv SHA-256
  92670074566e5f29a9e243540d39f608f6a3d6c64d6b061d991c88af335eba82.

- 2026-10-10T21:46:06+00:00: Recorded command exit 0; command argv SHA-256
  444e4c2dae1018f54837b3f39458afcc591b3a204d90ed5480c9b90b0e834a55.

- 2026-10-10T21:46:15+00:00: Recorded command exit 0; command argv SHA-256
  b6450fc1928ee39ccddd531049b97da67a0d55ecb6c887ddb58727298bb510cd.

- 2026-10-10T21:46:44+00:00: Recorded command exit 0; command argv SHA-256
  2016b3e8cd5775c42c403c3f1bd3ccc9b56f952d4183d8d11bf162bc53bf28b1.

- 2026-10-10T21:47:01+00:00: Recorded command exit 0; command argv SHA-256
  0675fd6c5cfdd230f79fdfd18cb7bd8f9686f6f0345be13ad93b7384545d5d69.

- 2026-10-10T21:47:08+00:00: Signed+DCO candidate 97eb1e38 repairs effective eight-attempt
  WouldBlock guard and adds retry-budget regression. Focused retry test, combined workspace gate,
  fmt, clippy and rustdoc pass. Release for independent review; no GitHub Reviews API approval
  object is required.

- 2026-10-10T21:47:18+00:00: Claimed by codex-ar1790-independent-review.

- 2026-10-10T21:47:21+00:00: Recorded command exit 0; command argv SHA-256
  934f85288ec94de317141997efe17d74a0af26181fbee3ec25bbdda7f8125ac7.

- 2026-10-10T21:47:34+00:00: Recorded command exit 0; command argv SHA-256
  98fc68ad8481c79d35b3883e9e5adb1ef4bda2c7027af2dc9d1e52b2837a4934.

- 2026-10-10T21:47:46+00:00: Recorded command exit 0; command argv SHA-256
  88db1695eada8f73d268303a8cd95b4cfe19be434e54f1d401c247c92bacfce8.

- 2026-10-10T21:47:53+00:00: Recorded command exit 0; command argv SHA-256
  84717dd68a830df01b192c9c719394f6d7b7d91ffd1b1cd69de2a98e87d1357f.

- 2026-10-10T21:48:23+00:00: Recorded command exit 0; command argv SHA-256
  8c1a2a9893e68acf6edc1830ddaf033770967be24ad25b5891eb5e66813546cb.

- 2026-10-10T21:48:52+00:00: Recorded command exit 0; command argv SHA-256
  d43e0ad394d932860975e92888092c2f49e32f45fcc7cfca8ca988b7c90ac158.

- 2026-10-10T21:48:59+00:00: Recorded command exit 0; command argv SHA-256
  294cd7057f55cbcb2ec5d90c74590e36395c24f714ff95fe17d78b5e01f838b9.

- 2026-10-10T21:49:09+00:00: Independent review blocks 97eb1e38: guard fix is semantically correct
  and focused metrics/control/rustup tests pass, but spawn_retry_budget_matches_declared_bound only
  tests helper predicate. It does not deterministically drive 4+ actual WouldBlock spawn failures or
  prove exhausted retries return typed ProbeRejected, as AR-1790 requires. No full qualification,
  PR, or merge.

- 2026-10-10T21:49:25+00:00: Claimed by codex-ar1790-independent-review.

- 2026-10-10T21:49:28+00:00: Independent review blocks 97eb1e38. Required repair: add deterministic
  actual RunningProcess spawn injection proving recovery after more than three WouldBlock failures
  and typed bounded ProbeRejected exhaustion; helper-only predicate coverage is insufficient.
  Focused metrics/control/rustup tests and signatures/DCO pass, but no full qualification, PR, or
  merge is authorized. No GitHub Reviews API approval object is required.

- 2026-10-10T21:49:50+00:00: Claimed by codex-ar1790-repair-coordinator.

- 2026-10-10T21:51:00+00:00: Recorded command exit 0; command argv SHA-256
  800a4baab9027da70e32cd5732f07bbe4ea594e839cc011e8b85a0231876a64a.

- 2026-10-10T21:51:08+00:00: Recorded command exit 1; command argv SHA-256
  791330aede9fb95c9d3a098359a95b4b5f7900d40fe0af5e822520b9e38ea607.

- 2026-10-10T21:51:21+00:00: Recorded command exit 0; command argv SHA-256
  e607e47b48559cc6e3bfe735b55c18b9e0ceaa90af3baf74f5de07a3e26a926f.

- 2026-10-10T21:51:35+00:00: Recorded command exit 0; command argv SHA-256
  d403f6f45bcc4f2b0763341fe1fe92ac311f4101f0a1bb06a28baf25fea4cf69.

- 2026-10-10T21:51:42+00:00: Recorded command exit 0; command argv SHA-256
  bad16b1c906c437f1ea743917cd2b1ab5e8264dd3b277c396f70f3000bbd9cf1.

- 2026-10-10T21:51:49+00:00: Recorded command exit 0; command argv SHA-256
  ea485257732edc5217d8107914edd5ffc1a57f5dd2a2ab06b89b45d376068ed5.

- 2026-10-10T21:51:53+00:00: Recorded command exit 0; command argv SHA-256
  8174d03d934fbd5a70c27efad7afac9f6a739b142cf42126c1b49620bce30d55.

- 2026-10-10T21:52:06+00:00: Recorded command exit 0; command argv SHA-256
  41d602fdf7b78269eb4850ad05239e66a8b5bc3b8b62dce7b9779619d53cc32d.

- 2026-10-10T21:53:03+00:00: Recorded command exit 0; command argv SHA-256
  444e4c2dae1018f54837b3f39458afcc591b3a204d90ed5480c9b90b0e834a55.

- 2026-10-10T21:53:16+00:00: Recorded command exit 0; command argv SHA-256
  2016b3e8cd5775c42c403c3f1bd3ccc9b56f952d4183d8d11bf162bc53bf28b1.

- 2026-10-10T21:53:28+00:00: Recorded command exit 0; command argv SHA-256
  0675fd6c5cfdd230f79fdfd18cb7bd8f9686f6f0345be13ad93b7384545d5d69.

- 2026-10-10T21:53:55+00:00: Repair commit c571ad8a adds deterministic actual-spawn WouldBlock
  recovery and typed exhaustion tests. Focused metrics, full workspace, fmt, clippy, and rustdoc
  gates pass. Release for fresh independent exact-head review; no GitHub Reviews API approval object
  is required.

- 2026-10-10T21:54:20+00:00: Claimed by codex-ar1790-independent-rereview.

- 2026-10-10T21:54:39+00:00: Recorded command exit 0; command argv SHA-256
  ad318822b5f4e63ec944e57b035620a87d2e8cb3db15935bce45c5081567fc27.

- 2026-10-10T21:55:19+00:00: Recorded command exit 0; command argv SHA-256
  ab750b75f47acbf962e8a81c2dc34e840b347dd4c86d92e3d81d6f51209866bd.

- 2026-10-10T21:55:34+00:00: Recorded command exit 0; command argv SHA-256
  bc616503ae09ebb44e22fcecb0f27518a0a44b59bb5f556546947cbf28fc7cb3.

- 2026-10-10T21:55:48+00:00: Recorded command exit 0; command argv SHA-256
  f524536c69739fb2f736d0bebdb01a3cb8a8f9734f140ba4c17c2dfc992f4026.

- 2026-10-10T21:56:20+00:00: Recorded command exit 0; command argv SHA-256
  3861261de8d3af7262e1946c256ea89615f43bc700add9bcaf9106ac651210be.

- 2026-10-10T21:56:33+00:00: Recorded command exit 0; command argv SHA-256
  4dec08e0302026efccae7c37152a14c03ee27848bfbde2c684241d1a18455739.

- 2026-10-10T21:57:38+00:00: Recorded command exit 0; command argv SHA-256
  87a9ee43fea803a4de3b3069bfecde7916b3038c6764611206ababe693ff1ce0.

- 2026-10-10T21:58:04+00:00: Recorded command exit 0; command argv SHA-256
  87a9ee43fea803a4de3b3069bfecde7916b3038c6764611206ababe693ff1ce0.

- 2026-10-10T21:58:07+00:00: Heartbeat by codex-ar1790-independent-rereview.

- 2026-10-10T21:58:55+00:00: Independent exact-head re-review passes for
  c571ad8a25774e0caea54a813939f27b66d5f2e8: SSH signature and matching DCO verified; diff --check
  and privacy inspection pass. Qualified actual-spawn recovery injects four WouldBlock failures then
  real-spawns successfully; qualified exhaustion makes exactly MAX_SPAWN_ATTEMPTS attempts and
  returns typed ProbeRejected. Focused metrics recovery/exhaustion, control-lock, and rustup
  path-replacement tests pass; two ordinary locked workspace runs pass. An initial unqualified
  filter ran zero tests and was immediately superseded by qualified exact tests. No GitHub Reviews
  API approval object is required. No merge authorized by this review; ready for integration
  decision.

- 2026-10-10T21:59:25+00:00: Claimed by codex-ar1790-integration.

- 2026-10-10T21:59:28+00:00: Recorded command exit 0; command argv SHA-256
  934f85288ec94de317141997efe17d74a0af26181fbee3ec25bbdda7f8125ac7.

- 2026-10-10T21:59:35+00:00: Recorded command exit 0; command argv SHA-256
  98fc68ad8481c79d35b3883e9e5adb1ef4bda2c7027af2dc9d1e52b2837a4934.

- 2026-10-10T21:59:42+00:00: Recorded command exit 0; command argv SHA-256
  f10d7bfa3114d59b954b041dc100fabbc87063758d526df95cd0a98d2e579c67.

- 2026-10-10T21:59:45+00:00: Recorded command exit 0; command argv SHA-256
  049633e008547c6b38fcbbe5310d0c1dd3205030834fff5290fa60128ad97f7e.

- 2026-10-10T21:59:55+00:00: Recorded command exit 0; command argv SHA-256
  868c1ed6f76f3557a6bd29facf9e421cad3e30682fefe344416954c640f6542e.

- 2026-10-10T22:00:06+00:00: Recorded command exit 0; command argv SHA-256
  ca84ba3618c80cf152a4f1a4e49da6dcbdb3a60c76cc3954cced8bb2d29b1f77.

- 2026-10-10T22:00:48+00:00: Recorded command exit 0; command argv SHA-256
  a19fb6c08b9b100acaa8db6f2632d122e173320acaf47bcf3b79ba2998dc293f.

- 2026-10-10T22:01:22+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:02:19+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:03:33+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:03:43+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:04:38+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:04:48+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:05:01+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:05:11+00:00: Recorded command exit 0; command argv SHA-256
  0883b05e109caca91dec8f66361f0b1423e6af27a0855cb32d66a474dda1691d.

- 2026-10-10T22:05:18+00:00: Recorded command exit 0; command argv SHA-256
  5207409309d8230ffec69801987a0d7a845914de1b5b05a020cf0f6fa7de6e94.

- 2026-10-10T22:05:26+00:00: Recorded command exit 0; command argv SHA-256
  69aae07284c79045914e7d9430eafa8248ba07106f6fab22c41b20fb8af7bad3.

- 2026-10-10T22:05:37+00:00: Heartbeat by codex-ar1790-integration.

- 2026-10-10T22:05:48+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:05:58+00:00: Recorded command exit 0; command argv SHA-256
  677905c244f48b08ab00500466ed046cdcf047f6a9defe8e66eccb66e2a72197.

- 2026-10-10T22:06:06+00:00: Recorded command exit 0; command argv SHA-256
  573c969c3ffac5aa77bbe28ac45a474f4889bb866d8f7dc7d7575edeb47ae05e.

- 2026-10-10T22:06:14+00:00: Recorded command exit 0; command argv SHA-256
  d577abe708b98640005fcadc832f435417af1c8869d0504fc7af33d921cb3c7c.

- 2026-10-10T22:06:23+00:00: Heartbeat by codex-ar1790-integration.

- 2026-10-10T22:06:31+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:06:41+00:00: Recorded command exit 0; command argv SHA-256
  9a8e58faf24b9ca5abbd788baae4e6f1aa1016beb98092925fbe5c0bb650f7fd.

- 2026-10-10T22:06:49+00:00: Recorded command exit 0; command argv SHA-256
  5c9627cddc60f73daa651e85b06d77eec199e21c2a9805c227a1cb78a16d4163.

- 2026-10-10T22:06:57+00:00: Recorded command exit 0; command argv SHA-256
  16f480fdff45c54b84d118757c5cda0bd749602f850cffe5c1bb8bbe892fb3ac.

- 2026-10-10T22:07:08+00:00: Heartbeat by codex-ar1790-integration.

- 2026-10-10T22:07:20+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:07:29+00:00: Heartbeat by codex-ar1790-integration.

- 2026-10-10T22:07:38+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:07:44+00:00: Heartbeat by codex-ar1790-integration.

- 2026-10-10T22:07:54+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:08:01+00:00: Recorded command exit 0; command argv SHA-256
  49f0f911c3b1558036b4cceca6a7e2ebdf665fa2a2509432e644ab4cb369f114.

- 2026-10-10T22:08:09+00:00: Recorded command exit 0; command argv SHA-256
  c600e6050ae827a089f07556d3472b29aed7dd6939bd113edfb02bbb50f0d6fb.

- 2026-10-10T22:08:17+00:00: Recorded command exit 0; command argv SHA-256
  664b246b4d36d0f563b142edd74839a94da170df149fdcaccbf3ede6e08a6505.

- 2026-10-10T22:08:29+00:00: Recorded command exit 8; command argv SHA-256
  e8da7cf4416fd958eb021bbcf4db02dab21649c419dc66ddd6542d2608045afa.

- 2026-10-10T22:08:38+00:00: Recorded command exit 0; command argv SHA-256
  49f0f911c3b1558036b4cceca6a7e2ebdf665fa2a2509432e644ab4cb369f114.

- 2026-10-10T22:08:45+00:00: Recorded command exit 0; command argv SHA-256
  c600e6050ae827a089f07556d3472b29aed7dd6939bd113edfb02bbb50f0d6fb.

- 2026-10-10T22:08:53+00:00: Recorded command exit 0; command argv SHA-256
  664b246b4d36d0f563b142edd74839a94da170df149fdcaccbf3ede6e08a6505.
