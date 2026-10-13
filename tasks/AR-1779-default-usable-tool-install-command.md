---
{
  "branch": "feature/ar-1779-default-usable-tool-install-command",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-13T01:56:36+00:00",
  "depends_on": [
    "AR-1777",
    "AR-1778",
    "AR-1784"
  ],
  "id": "AR-1779",
  "next_action": "AR-1825 is accepted on merged PR #622 / main b8fc46907afd281781a49dbd04ccb0aa6337a850: all 10 exact-main post-merge workflows are terminal success and source-executable matrix plus bounded adapter/rollback/smoke negative gates are present. AR-1779 release is blocked until current origin/main 356a669e0c6ebbb6cd3675a119cba16fea486283 is green; current unrelated AR-1775 follow-on has Portable protected-main provenance, Repository quality, and Rust verification failures. Do not touch AR-1826\u20131832.",
  "observed_branch": "repair/ar-1779-source-fallback-linear4",
  "observed_dirty": 0,
  "observed_head": "80060a79e716b42e021ddd9b116afdc560bc6cc1",
  "owner": "ar1779-default-install-luna",
  "plan": "../plans/AR-1779-default-usable-tool-install-command.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "integration-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1779.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1779.json",
  "spec_revision": 1,
  "status": "in_progress",
  "summary": "Make default asb tool install produce and verify a usable project-local supported executable tool without manual source copying.",
  "task_revision": 231,
  "title": "Default usable tool-install command",
  "updated_at": "2026-10-13T00:26:36+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1779-default-usable-tool-install-command"
}
---

Change the public experience to `asb tool install <supported-id> [--project PATH]`.
It resolves the catalog, reuses a compatible verified local binary when one is
available, otherwise retrieves a verified official prebuilt artifact, otherwise
uses the pinned source-build/dependency fallback. A success is valid only after
an executable smoke check proves the installed entrypoint can be used
by the target ASB adapter. Persist the complete receipt and active usable state
only after that check passes.

Keep explicit local-file or fixture installation as a clearly labelled
development/import mode rather than the default supported-user flow. Offer
`status`, `repair`, `update`, and `remove` semantics that expose current
catalog/source/digest/platform/build state without downloading on read-only
commands. Human output must explain selection, downloads/builds, reused versus
created dependencies, result location, and next action; JSON retains its stable
machine contract and never includes secrets. The command must not silently use a
wrong binary, silently fall back to an unrelated tool, or claim installation when
the adapter smoke check failed.

- 2026-10-11T13:53:29+00:00: Promote dependency-safe AR-1779 now that AR-1777, AR-1778, and AR-1784
  are durably done.

- 2026-10-11T13:54:34+00:00: Claimed by luna56-ar1779.

- 2026-10-11T13:55:27+00:00: Recorded command exit 0; command argv SHA-256
  ff6d1fa7a7185010f75fcfafe4bdc1bdd5b9804d8c2b3e7abdd8a1b96cd81c63.

- 2026-10-11T13:57:08+00:00: Recorded command exit 0; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-11T13:57:59+00:00: Recorded command exit 0; command argv SHA-256
  a15cc58cd42c65980b3f718472d4802c1ccec6ff105530e3f2089168c16c493a.

- 2026-10-11T13:58:50+00:00: Recorded command exit 0; command argv SHA-256
  a15cc58cd42c65980b3f718472d4802c1ccec6ff105530e3f2089168c16c493a.

- 2026-10-11T13:59:14+00:00: Recorded command exit 0; command argv SHA-256
  e274074a17e0682b5edf79b5caca577a41520ed9ca8daf9342f0433b678582a1.

- 2026-10-11T13:59:42+00:00: Recorded command exit 0; command argv SHA-256
  66f996973c66b1dc6463c1cf5f3af22fc2e3038ab84de35cb406f119798f0306.

- 2026-10-11T14:00:27+00:00: Recorded command exit 101; command argv SHA-256
  10a701b2424e5ed226b0455eb01ce19376ff98a24fb81ec0200c779f16f8a3dc.

- 2026-10-11T14:00:55+00:00: Recorded command exit 0; command argv SHA-256
  245e3b0fff1646dc3984ec302e56aa0b1995e88266374bb46e25680733a6a143.

- 2026-10-11T14:01:23+00:00: Recorded command exit 0; command argv SHA-256
  3ce7e7052283f0a20f77e43c8d9a7bbdd6c377538c05e7f56ee034002add5ad6.

- 2026-10-11T14:02:37+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-11T14:03:07+00:00: Recorded command exit 0; command argv SHA-256
  e274074a17e0682b5edf79b5caca577a41520ed9ca8daf9342f0433b678582a1.

- 2026-10-11T14:03:33+00:00: Recorded command exit 0; command argv SHA-256
  9bee9533393737c3f6f9394ca04410e19296da8940afaf99aaad5895237504ef.

- 2026-10-11T14:03:58+00:00: Recorded command exit 0; command argv SHA-256
  f18ad62f9c9d9a6760e09d95ece99cc58b831e4016946fa857d41e1688516b8c.

- 2026-10-11T14:04:44+00:00: Recorded command exit 0; command argv SHA-256
  cab79174958809e6e46c45ff4e7953c404de8b4e7dcba8de94d226f3ce8d4ac8.

- 2026-10-11T14:05:22+00:00: Rustfmt repair 32cb692a pushed. Full cargo test -p asb-cli passed
  before repair; repair is formatting-only. PR #583 exact head updated.

- 2026-10-11T14:08:25+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-11T14:09:07+00:00: Recorded command exit 0; command argv SHA-256
  e274074a17e0682b5edf79b5caca577a41520ed9ca8daf9342f0433b678582a1.

- 2026-10-11T14:09:29+00:00: Recorded command exit 0; command argv SHA-256
  f6b6f842c448a90c56fe8c665d4f473b301b67152ad8b1849254c6ae566946c2.

- 2026-10-11T14:09:54+00:00: Recorded command exit 0; command argv SHA-256
  f18ad62f9c9d9a6760e09d95ece99cc58b831e4016946fa857d41e1688516b8c.

- 2026-10-11T14:10:16+00:00: Pushed signed repair 7c08cfb0. Added explicit lifecycle repair/update
  installed-ID validation, transactional rerun behavior, and human/machine
  selection/source/build/reuse/path/next fields. Focused parser test passes.

- 2026-10-11T14:10:32+00:00: Heartbeat by luna56-ar1779.

- 2026-10-11T14:11:42+00:00: Recorded command exit 0; command argv SHA-256
  a3fcf50559681de15238986360c1d4702b78883aa98c68d9afd55eca44a0b8e2.

- 2026-10-11T14:11:53+00:00: Recorded command exit 0; command argv SHA-256
  5cf7ef1b33d2d66d11d63bbe78d94081f0cb3366cd0a77c841993395ec8f82a9.

- 2026-10-12T13:21:50+00:00: Recovered expired claim formerly owned by luna56-ar1779. Expired owner
  has no live worker process; recover for coordinator consistency.

- 2026-10-12T17:11:15+00:00: Claimed by ar1779-default-install-luna.

- 2026-10-12T17:11:22+00:00: Recorded command exit 0; command argv SHA-256
  6129ccfce874b9c6a080b6553d85151682cf6a1e68da4436564c985d70b81d27.

- 2026-10-12T17:12:36+00:00: Recorded command exit 0; command argv SHA-256
  630a47c5dda3f440a42206c54f51d033e82fe32668ae4851ef6bd44774bcf5c9.

- 2026-10-12T17:13:31+00:00: Recorded command exit 0; command argv SHA-256
  765ab8a28d928e91c08eca048aedf076b27901f254016a3d76775abdccaf3d99.

- 2026-10-12T17:13:43+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:13:48+00:00: Recorded command exit 1; command argv SHA-256
  073537e3e58a40a792d6295de536b0563f1d67fd832f49bef085db60821e8dd1.

- 2026-10-12T17:13:57+00:00: Recorded command exit 0; command argv SHA-256
  630a47c5dda3f440a42206c54f51d033e82fe32668ae4851ef6bd44774bcf5c9.

- 2026-10-12T17:14:01+00:00: Recorded command exit 0; command argv SHA-256
  4ec53513304e95c3e28a989d902ade2abb597d249af1b8b2ebab6dd7285885d5.

- 2026-10-12T17:14:28+00:00: Recorded command exit 0; command argv SHA-256
  b39ca4c5544dfedb640d844bb55ef0c8de36c5d8d48093ada327f8b9e7d4282a.

- 2026-10-12T17:14:49+00:00: Recorded command exit 0; command argv SHA-256
  42f52f2ec3ac69b9c6003b4ad6846f961979646df01a2180f045c085e92c9f6b.

- 2026-10-12T17:15:10+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:15:13+00:00: Recorded command exit 0; command argv SHA-256
  4d94e1c9d16ce91bd609909f42b81fd617d1ad62144380ed0bcc223f3f35deb4.

- 2026-10-12T17:15:18+00:00: Recorded command exit 0; command argv SHA-256
  e274074a17e0682b5edf79b5caca577a41520ed9ca8daf9342f0433b678582a1.

- 2026-10-12T17:15:28+00:00: Recorded command exit 0; command argv SHA-256
  fa84920ff03dc0a2e85328643b0f9073f02231d622d2f3365bd70fe60b64385a.

- 2026-10-12T17:15:43+00:00: Recorded command exit 0; command argv SHA-256
  bc768d469b09cb617aad74f023976a0a3a07690ea56cfce6dca19c48e2932243.

- 2026-10-12T17:15:56+00:00: Replaced stale PR #583 next action: its implementation was superseded
  by merged PR #585. Current-main repair adds receipt-validated catalog reuse and bounded
  credential-free entrypoint smoke verification; focused tests pass.

- 2026-10-12T17:16:17+00:00: Recorded command exit 101; command argv SHA-256
  f4ca587d92ce889a616cbf0ab1c3902483f2596853ed2e353532e6413afd3ca9.

- 2026-10-12T17:16:26+00:00: Recorded command exit 0; command argv SHA-256
  095e6706d3fd1d9bde496b81c2db452a606e5b1e20ab5f447f86b5fc07919e55.

- 2026-10-12T17:16:37+00:00: Recorded command exit 0; command argv SHA-256
  e274074a17e0682b5edf79b5caca577a41520ed9ca8daf9342f0433b678582a1.

- 2026-10-12T17:16:45+00:00: Recorded command exit 0; command argv SHA-256
  94060f205e325aacbbd2e1730c052f4f938370a5f35552f93b0bc6d18efc1595.

- 2026-10-12T17:17:02+00:00: Recorded command exit 0; command argv SHA-256
  f4ca587d92ce889a616cbf0ab1c3902483f2596853ed2e353532e6413afd3ca9.

- 2026-10-12T17:17:37+00:00: Recorded command exit 101; command argv SHA-256
  8451e7fa7fe09018adb31cdfa413e4fb168ad8aa0345923d3fd7e0f7b625693b.

- 2026-10-12T17:18:52+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:19:01+00:00: Current-main repair is product commit
  a37da01d4d3557122dbe8b3bd595cc9de9b2826e (SSH signature valid, matching DCO); focused tests and
  clippy passed. Stale PR #583 branch remains at daf2d3ee and is not being force-updated.

- 2026-10-12T17:19:15+00:00: Recorded command exit 0; command argv SHA-256
  b3fc1eb1fa92c713fda4d9d88902a0e795fe9397c95a80d326a4840681733ba1.

- 2026-10-12T17:19:32+00:00: Recorded command exit 0; command argv SHA-256
  2f9cc88b200a8649751aafbd23dee5f1dc7bd1761cd333ab8b879ef61b9d75fc.

- 2026-10-12T17:19:49+00:00: Recorded command exit 0; command argv SHA-256
  4ad78f884eedc4a0bb3442b2a0fc4cd8df21b956f6fe049bb0413531b88eafe8.

- 2026-10-12T17:20:02+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:20:44+00:00: Recorded command exit 0; command argv SHA-256
  a63d7df09166ff1c467a7bf4582f5ad38227203a2ff00aa9802ac4b575c6e577.

- 2026-10-12T17:20:59+00:00: Recorded command exit 0; command argv SHA-256
  a63d7df09166ff1c467a7bf4582f5ad38227203a2ff00aa9802ac4b575c6e577.

- 2026-10-12T17:21:19+00:00: PR #618 is published cleanly from repair/ar-1779-current-main-smoke;
  stale PR #583 and its branch were preserved, never force-updated. Current-head checks: Kani, Loom,
  retained faults, platform evidence, provenance, headers, and AWQ shadow green; Rust,
  credential-free, aarch64, policy/coverage, fuzz/mutation, and TUI checks still running.

- 2026-10-12T17:21:59+00:00: Recorded command exit 0; command argv SHA-256
  a63d7df09166ff1c467a7bf4582f5ad38227203a2ff00aa9802ac4b575c6e577.

- 2026-10-12T17:22:09+00:00: Recorded command exit 0; command argv SHA-256
  a63d7df09166ff1c467a7bf4582f5ad38227203a2ff00aa9802ac4b575c6e577.

- 2026-10-12T17:22:24+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:22:33+00:00: Recorded command exit 8; command argv SHA-256
  fb59ad09e0c5642e05f534aac856e041a170286383d3fcbb33e0807ec786c3cf.

- 2026-10-12T17:22:57+00:00: Recorded command exit 8; command argv SHA-256
  fb59ad09e0c5642e05f534aac856e041a170286383d3fcbb33e0807ec786c3cf.

- 2026-10-12T17:23:12+00:00: Recorded command exit 0; command argv SHA-256
  435dc7484f2b8f855cf3dd51906e0f2a1605f4c0288244a9380b3061a824e1b3.

- 2026-10-12T17:23:23+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:26:19+00:00: Recorded command exit 0; command argv SHA-256
  83dc474c9e7276d30d924b93c058053d95fbb067765b912dfd0a06f3a8f4eb9a.

- 2026-10-12T17:27:36+00:00: Recorded command exit 101; command argv SHA-256
  5c60647d42385cfdba0de784a4eeb26d886df06de34dc4f6043f1570fc5bc3f3.

- 2026-10-12T17:28:36+00:00: Independent review rejected exact head
  a37da01d4d3557122dbe8b3bd595cc9de9b2826e with three concrete findings; diagnostic_contract
  currently reproduces shifted-callsite failure. PR #618 remains unmergeable and will be updated
  only after repair.

- 2026-10-12T17:29:48+00:00: Recorded command exit 101; command argv SHA-256
  6d2ff482e7ecd968412864b00486eef0e534e806de8cca335fa2a2f5e7b3c976.

- 2026-10-12T17:30:40+00:00: Recorded command exit 101; command argv SHA-256
  5c60647d42385cfdba0de784a4eeb26d886df06de34dc4f6043f1570fc5bc3f3.

- 2026-10-12T17:30:54+00:00: Recorded command exit 101; command argv SHA-256
  5c60647d42385cfdba0de784a4eeb26d886df06de34dc4f6043f1570fc5bc3f3.

- 2026-10-12T17:31:27+00:00: Recorded command exit 0; command argv SHA-256
  5c60647d42385cfdba0de784a4eeb26d886df06de34dc4f6043f1570fc5bc3f3.

- 2026-10-12T17:31:53+00:00: Recorded command exit 101; command argv SHA-256
  65660344b5d85b42a2b59bc94e4d807b381b4c8e2cc01201505bb137cf3dd111.

- 2026-10-12T17:32:29+00:00: Recorded command exit 101; command argv SHA-256
  65660344b5d85b42a2b59bc94e4d807b381b4c8e2cc01201505bb137cf3dd111.

- 2026-10-12T17:32:54+00:00: Recorded command exit 1; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-12T17:33:23+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T17:33:51+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:34:02+00:00: Recorded command exit 0; command argv SHA-256
  e0746398d13a6248d682a0ee0cdde1b986251f2c01d5b08a2db90f1214e70d36.

- 2026-10-12T17:34:16+00:00: Recorded command exit 0; command argv SHA-256
  eb8648d78be9d44db5631385c3fcbaaeaac0114989924fef6dc1415045ce0bfa.

- 2026-10-12T17:34:35+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:35:18+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-12T17:35:41+00:00: Recorded command exit 0; command argv SHA-256
  7109951d44fa989265b6b6481cadfc8b40676ae39f4e04b8e1126fe99b431b01.

- 2026-10-12T17:36:26+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-12T17:36:38+00:00: Recorded command exit 0; command argv SHA-256
  ef19043205064ea87bec66574bfe246191c9311bc8c404cd8e347d08fbdc44f0.

- 2026-10-12T17:37:56+00:00: Recorded command exit 0; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-12T17:38:31+00:00: Recorded command exit 0; command argv SHA-256
  b3fc1eb1fa92c713fda4d9d88902a0e795fe9397c95a80d326a4840681733ba1.

- 2026-10-12T17:39:24+00:00: Repaired review findings: typed diagnostic identities plus explicit
  reviewed insertion-offset allowlist, receipt verifier included and positive/negative reuse test,
  source builder rejects package archives/non-executable output before activation and validates
  executable staging. Product commits b76659a8, ddb0d931, db08ffb3 are SSH-signed with DCO. PR #618
  updated without force-pushing stale PR #583.

- 2026-10-12T17:39:31+00:00: Recorded command exit 0; command argv SHA-256
  a63d7df09166ff1c467a7bf4582f5ad38227203a2ff00aa9802ac4b575c6e577.

- 2026-10-12T17:39:41+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:39:46+00:00: Recorded command exit 8; command argv SHA-256
  fb59ad09e0c5642e05f534aac856e041a170286383d3fcbb33e0807ec786c3cf.

- 2026-10-12T17:39:59+00:00: Recorded command exit 0; command argv SHA-256
  5e8a3e7119953a89ccab32353dfa91a7cd573eae08e32816d1f4a000e93ab598.

- 2026-10-12T17:40:10+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:40:16+00:00: Recorded command exit 8; command argv SHA-256
  fb59ad09e0c5642e05f534aac856e041a170286383d3fcbb33e0807ec786c3cf.

- 2026-10-12T17:40:58+00:00: Recorded command exit 8; command argv SHA-256
  fb59ad09e0c5642e05f534aac856e041a170286383d3fcbb33e0807ec786c3cf.

- 2026-10-12T17:41:01+00:00: Recorded command exit 8; command argv SHA-256
  fb59ad09e0c5642e05f534aac856e041a170286383d3fcbb33e0807ec786c3cf.

- 2026-10-12T17:41:43+00:00: Recorded command exit 8; command argv SHA-256
  fb59ad09e0c5642e05f534aac856e041a170286383d3fcbb33e0807ec786c3cf.

- 2026-10-12T17:42:41+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:43:41+00:00: Second independent review approved diagnostic and receipt repairs but
  rejected source fallback because package archives were treated as entrypoints and
  openjiuwen/perf/bpftool lacked build recipes. Repair design: retain pinned identities as metadata,
  remove all source fallback from default install until a verified native/launcher adapter exists,
  add a specific actionable diagnostic, and enforce executable headers in source_builder with
  positive/negative tests.

- 2026-10-12T17:43:47+00:00: Recorded command exit 1; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-12T17:44:12+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-12T17:44:37+00:00: Recorded command exit 1; command argv SHA-256
  422f79d615e5b979c6ee13fb7ec34f076339fe84a70a73bcc5dd961a1302b431.

- 2026-10-12T17:45:19+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:47:38+00:00: Recovered interrupted worker with no live process/session; preserved PR
  #618 head db08ffb3 and recorded repair next action. Replacement may claim after reconciliation.

- 2026-10-12T17:49:46+00:00: Claimed by ar1779-default-install-luna.

- 2026-10-12T17:50:13+00:00: Recorded command exit 0; command argv SHA-256
  7c730eec3ea71bd24ffc5a9255bf7f542481bf601dba6c72bed6e00f6c2fab3c.

- 2026-10-12T17:50:34+00:00: Recorded command exit 1; command argv SHA-256
  364106021e4fe8a60023e128d8cf6a3bdb8bcda46fcbcdc0f77a49606e61c5b9.

- 2026-10-12T17:51:02+00:00: Recorded command exit 1; command argv SHA-256
  0579439a76d5f1c46e1d3248cbe06c4e641b8233a69d33cce10d18045cdf1c7f.

- 2026-10-12T17:51:15+00:00: Recorded command exit 0; command argv SHA-256
  c9a28b4739a50bd84129f11c53b6bae84706bd7fe443280a34847d4be115b486.

- 2026-10-12T17:51:25+00:00: Recorded command exit 0; command argv SHA-256
  fe0bed0d93ea42d26fe3344bc3187029ee8b3a177b14f7cf6c20286ca4b230cf.

- 2026-10-12T17:51:36+00:00: Recorded command exit 0; command argv SHA-256
  e15288e1a6e1a52d76181852314c98e2461663dfdb356d0c7847e6901c96c119.

- 2026-10-12T17:51:48+00:00: Recorded command exit 101; command argv SHA-256
  666afb1c7d0718c00cba52d0375cd8cee7481d6630de90166ac257fc199399cb.

- 2026-10-12T17:51:57+00:00: Recorded command exit 1; command argv SHA-256
  8d0e7ef5e58821b67270c184ea93f58bc281ef738408121bf2d72196689f8801.

- 2026-10-12T17:52:07+00:00: Recorded command exit 0; command argv SHA-256
  7d448ee9f0a90b77255f872ff15ad76b8301c57b2472bc2eea6d28aa62ac2ff4.

- 2026-10-12T17:52:21+00:00: Recorded command exit 101; command argv SHA-256
  f1f34056a671772b989c1e58f18c3b3e5f3d138fbc7fcfafdc48a299e7492aab.

- 2026-10-12T17:53:07+00:00: Recorded command exit 0; command argv SHA-256
  46314d0a775dbd38948faef2897de7e3ee639d5f0375bd979b8622ece0da4c5f.

- 2026-10-12T17:53:25+00:00: Recorded command exit 0; command argv SHA-256
  1e3ce384d8b13d51eeb1e12ea15aa23407beed578d2a67e143eeb7bf052df922.

- 2026-10-12T17:54:09+00:00: Recorded command exit 1; command argv SHA-256
  aa6158bc0833c5a0507f6b8352513bec8907d8dc9bc7f65a208087c38663ce14.

- 2026-10-12T17:54:12+00:00: Recorded command exit 101; command argv SHA-256
  23563cf4f6d732fced8d0fe5c9443d1d650e0ae0ce6f0f9de8247d24fe1e5d97.

- 2026-10-12T17:54:23+00:00: Recorded command exit 0; command argv SHA-256
  c1167594bdb3735df45ce9e7a8625b4b0a1ab520db5399ee60aaaabeb342ad2a.

- 2026-10-12T17:54:42+00:00: Recorded command exit 0; command argv SHA-256
  af4123939a22f16673dc155f33d87e49f9a17ab9b23345e921a323839d7058cf.

- 2026-10-12T17:55:20+00:00: Recorded command exit 101; command argv SHA-256
  23563cf4f6d732fced8d0fe5c9443d1d650e0ae0ce6f0f9de8247d24fe1e5d97.

- 2026-10-12T17:55:32+00:00: Recorded command exit 0; command argv SHA-256
  91287a9c791861e8ca5201c493486c91df6edaaa61a38fbaf81d70d4d7984fb9.

- 2026-10-12T17:55:42+00:00: Recorded command exit 0; command argv SHA-256
  39d08baabc9737d9d31b4a55bcdec58dcd8620695511c53bc4a7b413b3c49d41.

- 2026-10-12T17:55:53+00:00: Recorded command exit 0; command argv SHA-256
  cf87e7eaa8983b2011430ea72e8b0e97e19081979b6d342f19d03f0c8378b207.

- 2026-10-12T17:56:07+00:00: Recorded command exit 0; command argv SHA-256
  aa6158bc0833c5a0507f6b8352513bec8907d8dc9bc7f65a208087c38663ce14.

- 2026-10-12T17:56:22+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T17:56:32+00:00: Source fallback now fails closed for non-executable source/archive
  catalog entries with typed catalog_source_adapter_unavailable; source adapter header checks and
  reconciled diagnostic identities pass focused/full asb-cli gates.

- 2026-10-12T17:56:49+00:00: Recorded command exit 0; command argv SHA-256
  c5ba738a0efa3f642ee08c1608f448324ef86656753bdf405cd85698fb495dc6.

- 2026-10-12T17:57:29+00:00: Exact CI failure recorded: merge commit 309dc039 introduced daf2d3ee as
  a non-first-parent range; do not merge or force-push this branch.

- 2026-10-12T17:57:44+00:00: Recorded command exit 0; command argv SHA-256
  7c67220b578ee248f7bdf8065befcd1530454fe558210d16b6b74d3c437005aa.

- 2026-10-12T17:58:12+00:00: Recorded command exit 1; command argv SHA-256
  c2127d26cd20d35d786db48d60bf8ef2d5ff891f647d197dba3621015ec17341.

- 2026-10-12T17:58:30+00:00: Recorded command exit 0; command argv SHA-256
  4f566193afc33f4e59efdb1d9cb6ec9ec185e2c7756bd8327f1a7a4227b65f43.

- 2026-10-12T17:58:44+00:00: Recorded command exit 128; command argv SHA-256
  e93455ea461d4bf7046d4397cc9b05c85c099e60812b4ca26be4e04d33a9b87b.

- 2026-10-12T17:58:58+00:00: Recorded command exit 128; command argv SHA-256
  fb17664ff80757e821b4297280f43b95ef0416704c291da3c031e1ea830f86b1.

- 2026-10-12T17:59:13+00:00: Recorded command exit 1; command argv SHA-256
  6e95e6161d5dc9e6689e172c134426f8e53def70bf78c40f75fa38304fbf710a.

- 2026-10-12T18:00:01+00:00: Recorded command exit 0; command argv SHA-256
  fe0bed0d93ea42d26fe3344bc3187029ee8b3a177b14f7cf6c20286ca4b230cf.

- 2026-10-12T18:00:16+00:00: Recorded command exit 1; command argv SHA-256
  e93455ea461d4bf7046d4397cc9b05c85c099e60812b4ca26be4e04d33a9b87b.

- 2026-10-12T18:00:35+00:00: Recorded command exit 0; command argv SHA-256
  39d08baabc9737d9d31b4a55bcdec58dcd8620695511c53bc4a7b413b3c49d41.

- 2026-10-12T18:00:46+00:00: Recorded command exit 0; command argv SHA-256
  e93455ea461d4bf7046d4397cc9b05c85c099e60812b4ca26be4e04d33a9b87b.

- 2026-10-12T18:01:00+00:00: Recorded command exit 0; command argv SHA-256
  c9a28b4739a50bd84129f11c53b6bae84706bd7fe443280a34847d4be115b486.

- 2026-10-12T18:01:15+00:00: Recorded command exit 101; command argv SHA-256
  23563cf4f6d732fced8d0fe5c9443d1d650e0ae0ce6f0f9de8247d24fe1e5d97.

- 2026-10-12T18:01:34+00:00: Recorded command exit 1; command argv SHA-256
  ffa4611f84ee305f7d7722eb4e6cfb699d827fa5da38b63cdf69d88ebdffbf2c.

- 2026-10-12T18:01:57+00:00: Recorded command exit 0; command argv SHA-256
  395721bfcb29ee33f5a57198eeee07922377578c3e2cfd6b446f255bfa9599b0.

- 2026-10-12T18:02:08+00:00: Recorded command exit 0; command argv SHA-256
  ab9a825e9476da6c1dca8fca471c1fd3deaccbed3fc454c60154fb0a7e679e5c.

- 2026-10-12T18:02:20+00:00: Recorded command exit 0; command argv SHA-256
  75c8a79da16905f7896d0d54614141eb5bda40fac3584ba4388c40b653d60912.

- 2026-10-12T18:02:33+00:00: Recorded command exit 0; command argv SHA-256
  e93455ea461d4bf7046d4397cc9b05c85c099e60812b4ca26be4e04d33a9b87b.

- 2026-10-12T18:03:10+00:00: Recorded command exit 0; command argv SHA-256
  ea137053d2f500075701e97c3c6c57206fac609fab1dddf830c073961ad8e846.

- 2026-10-12T18:03:24+00:00: Recorded command exit 0; command argv SHA-256
  208ef96323dad3aab2044ae0342397138650ec625001db1ce0a09f67b7e6a505.

- 2026-10-12T18:04:05+00:00: Recorded command exit 0; command argv SHA-256
  23563cf4f6d732fced8d0fe5c9443d1d650e0ae0ce6f0f9de8247d24fe1e5d97.

- 2026-10-12T18:04:13+00:00: Recorded command exit 0; command argv SHA-256
  37d22c542864e938a37a43b7b4eac607d3aaf8310e12f64fd4d4129e17fd4360.

- 2026-10-12T18:04:37+00:00: Rebased repair linearly from db08ffb3; full asb-cli suite passes. Prior
  merge-based f1dbe055 remains rejected by portable protected-main provenance and is not to be
  merged.

- 2026-10-12T18:05:21+00:00: Recorded command exit 0; command argv SHA-256
  e1bcca5b81010d249dd0cf7b783203e8308e1159c8e884089d5ef7376712c8c4.

- 2026-10-12T18:05:53+00:00: Created replacement PR
  https://github.com/martin-beck/agent-systems-benchmark/pull/619 from
  repair/ar-1779-source-fallback-linear3. Full asb-cli gates passed at 21372a22.

- 2026-10-12T18:07:03+00:00: Recorded command exit 0; command argv SHA-256
  0eedf3071659641810b028790aa5997f6409b523d75d089cef51a176df9c53a3.

- 2026-10-12T18:07:27+00:00: Recorded command exit 0; command argv SHA-256
  363635ed6f07bf5355f4a7ea14be10fb0b002a2715b71b5f34ff6af69609d755.

- 2026-10-12T18:07:43+00:00: Recorded command exit 0; command argv SHA-256
  fe0bed0d93ea42d26fe3344bc3187029ee8b3a177b14f7cf6c20286ca4b230cf.

- 2026-10-12T18:07:53+00:00: Recorded command exit 0; command argv SHA-256
  1d15884f8ab722170f4b014c00ea205df4c25203b3e2070216e9ae1d4507e574.

- 2026-10-12T18:08:05+00:00: Recorded command exit 0; command argv SHA-256
  e6f5931392c29beb24d0f3fa69e4e52bd8cfa215f61e8c65b026523c525dd30e.

- 2026-10-12T18:08:17+00:00: Recorded command exit 0; command argv SHA-256
  3049986e0559691be341e8e042329cd6e97cdd410ab7cd00685d01bbbc90c562.

- 2026-10-12T18:08:28+00:00: Recorded command exit 0; command argv SHA-256
  fd1ef2ae6bbf82c4a86a0aa3aee63aab809e1c197cba4119c4103e931975948d.

- 2026-10-12T18:08:40+00:00: Recorded command exit 1; command argv SHA-256
  ae3024de8778e8564a65b396be90cab411342bfd6ebc870e2b42ca5a6bb82a8c.

- 2026-10-12T18:08:55+00:00: Recorded command exit 0; command argv SHA-256
  c9a28b4739a50bd84129f11c53b6bae84706bd7fe443280a34847d4be115b486.

- 2026-10-12T18:09:32+00:00: Recorded command exit 0; command argv SHA-256
  23563cf4f6d732fced8d0fe5c9443d1d650e0ae0ce6f0f9de8247d24fe1e5d97.

- 2026-10-12T18:09:38+00:00: Recorded command exit 0; command argv SHA-256
  b7c40b284d373dace20c563ebc78d6ff6fc3dec4b2816dd1485f6edd5e358028.

- 2026-10-12T18:09:50+00:00: Recorded command exit 0; command argv SHA-256
  32f26525e0b0a6bff5ac3f13b9d2ec4fba81c966c2f46b8f139e15d96e7daa0a.

- 2026-10-12T18:10:09+00:00: Created replacement PR
  https://github.com/martin-beck/agent-systems-benchmark/pull/620 from
  repair/ar-1779-source-fallback-linear4. Source-only/archive/no-recipe IDs are explicitly excluded
  from default-supported install while retained for development/import; successor must define
  bounded source-to-executable adapter materialization, runtime/dependency contract, archive
  extraction, smoke proof, and retention semantics.

- 2026-10-12T18:11:23+00:00: Recorded command exit 1; command argv SHA-256
  062287a4a156bf57e73c2cd1616d9bc0178aab578677372a61c4b078735d3532.

- 2026-10-12T18:13:55+00:00: Linked successor AR-1825 for the unresolved source-to-executable
  adapter/runtime gap; AR-1779 remains pending until AR-1825 is accepted.

- 2026-10-12T18:22:32+00:00: Recorded command exit 0; command argv SHA-256
  deca3ed4bedc235ed8857e408391e5b6de880c2e024c068f54110f4584b27567.

- 2026-10-12T18:22:55+00:00: Recorded command exit 0; command argv SHA-256
  c20564cfbc869d1cacba08bb15d9ff05822c59a199a75d3168c7da4d7d1ebf5c.

- 2026-10-12T18:23:10+00:00: Recorded command exit 0; command argv SHA-256
  9bf234266addea9bcfbfb410f25c02f1554e180c74ca2acdcbd9baca36705778.

- 2026-10-12T18:23:29+00:00: Recorded command exit 0; command argv SHA-256
  05814e582e8d4bcaa3ab570b85a1580a862f0841cbef6670b5f85360d639e98b.

- 2026-10-12T18:23:45+00:00: Recorded command exit 1; command argv SHA-256
  9bf234266addea9bcfbfb410f25c02f1554e180c74ca2acdcbd9baca36705778.

- 2026-10-12T18:24:01+00:00: Recorded command exit 0; command argv SHA-256
  14e2da29686cca5799f26fbbe1b95cb16c987ade65773dbded7f33196caa3101.

- 2026-10-12T18:24:14+00:00: Recorded command exit 0; command argv SHA-256
  05814e582e8d4bcaa3ab570b85a1580a862f0841cbef6670b5f85360d639e98b.

- 2026-10-12T18:24:28+00:00: Recorded command exit 0; command argv SHA-256
  8d2c93161190a70427f6ec9eab8ab92f44cd29bd7f45d4da36950647e82dbc94.

- 2026-10-12T18:24:42+00:00: Recorded command exit 0; command argv SHA-256
  743dea445286d62043393ceaf0ba9b9b91f0634b4964b7e82fb0e1514bb9ec58.

- 2026-10-12T18:37:11+00:00: Recorded command exit 2; command argv SHA-256
  db7b4329dc1509c1c95b23da0e6cbb9072081ed6dd8ce5211c4b5cd5c722ce9e.

- 2026-10-12T18:37:36+00:00: Recorded command exit 1; command argv SHA-256
  08b6c951fc3bec19b3c84300177f1bc6b671d48478f4b7374fdba5fa0904b1cd.

- 2026-10-12T18:37:52+00:00: Recorded command exit 0; command argv SHA-256
  7d8e94ebcfd2759bb0e1d53ec4f1fff919d39f364271103e39ebe493d5c753fe.

- 2026-10-12T18:38:19+00:00: Recorded command exit 0; command argv SHA-256
  17d67f193fec412c5d85aba61f66d2e1bae2e1299c065bc20f2e648c1dda4c96.

- 2026-10-12T18:45:22+00:00: PR #620 safety repair superseded by clean PR #621 provenance forward
  repair; exact PR checks green, merge_pr.py published two-parent merge ab0a791b, portable
  provenance green; post-merge workflows remain in progress.

- 2026-10-12T18:48:34+00:00: Final evidence: PR #620 safety scope replaced by PR #621 forward DCO
  provenance repair; exact-head PR #621 b00cdba5 all required checks green; merge_pr.py accepted
  two-parent merge ab0a791b05d6082fd0c99ab287f8a7883180bcec; exact main post-merge workflows all
  terminal success including Rust, repository quality, aarch64, policy/provenance, formal, fault,
  credential-free, broker, hosted portability, and headers; reconcile --commit --push and doctor
  --live OK. AR-1825 remains prerequisite for genuine source adapter/runtime support.

- 2026-10-12T20:36:51+00:00: Recovered expired claim formerly owned by ar1779-default-install-luna.
  Recover stale expired ownership before promoting dependency-safe AR-1783; preserve in-progress
  AR-1779 evidence and owner history.

- 2026-10-12T22:23:17+00:00: Claimed by ar1779-default-install-luna.

- 2026-10-12T22:23:44+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T22:23:56+00:00: Acceptance reconciliation: PR #621 merged as
  ab0a791b05d6082fd0c99ab287f8a7883180bcec with verified SSH+DCO and 10/10 exact-main workflows
  green. Successor AR-1825 is done with mechanical spec acceptance
  sha256:00a3aa982989ac4d8face264857686f1cff9729e8707138b56b51d158e18e5ef, merged PR #622
  b8fc46907afd281781a49dbd04ccb0aa6337a850, and 10/10 exact-main post-merge workflows green. Matrix
  explicitly supports only qualified pinned-prebuilt rows and marks source-only/archive/no-recipe
  rows unsupported; adapter contract rejects unknown strategy, identity/runtime/entrypoint
  violations, bounded smoke failures, and transactional false usable state. Current origin/main
  advanced to 356a669e after AR-1775 and has 3 required workflow failures, so release cannot
  truthfully claim green current main. Preserve AR-1826–1832.

- 2026-10-12T22:24:33+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T22:24:52+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-12T22:25:07+00:00: Heartbeat by ar1779-default-install-luna.

- 2026-10-13T00:25:35+00:00: Recover expired ownerless state after lease elapsed; preserve branch
  and evidence for restart.

- 2026-10-13T00:26:19+00:00: Claimed by ar1779-default-install-luna.

- 2026-10-13T00:26:36+00:00: Heartbeat by ar1779-default-install-luna.
