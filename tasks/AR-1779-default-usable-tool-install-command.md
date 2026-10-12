---
{
  "branch": "feature/ar-1779-default-usable-tool-install-command",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-12T19:34:35+00:00",
  "depends_on": [
    "AR-1777",
    "AR-1778",
    "AR-1784"
  ],
  "id": "AR-1779",
  "next_action": "PR #618 now points to repaired signed head db08ffb3; request fresh independent review of all three findings and await exact-head hosted CI. Full locked workspace tests, diagnostic contract, catalog reuse tests, clippy, and fmt are green. Merge only after independent approval and all required checks, then post-merge requalify source/prebuilt install paths.",
  "observed_branch": "feature/ar-1779-default-usable-tool-install-command",
  "observed_dirty": 0,
  "observed_head": "db08ffb341d5bb8666c8aa9119d01503bc458b26",
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
  "task_revision": 103,
  "title": "Default usable tool-install command",
  "updated_at": "2026-10-12T17:39:24+00:00",
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
