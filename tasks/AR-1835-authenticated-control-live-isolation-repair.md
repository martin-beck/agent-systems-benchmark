---
{
  "branch": "feature/ar-1835-authenticated-control-live-isolation-repair",
  "checkpoint_commit": "06565d3de95aa72fa67bf2fde73078753acfd49b",
  "claim_expires": "",
  "depends_on": [
    "AR-1763",
    "AR-1764",
    "AR-1782"
  ],
  "id": "AR-1835",
  "next_action": "Remain blocked from merge on repeated hosted TUI qualification failure. Preserve exact logs and local-pass evidence; escalate as external asb-tui/main qualification or hosted-environment failure. No PR commit or merge until the required TUI check is independently resolved and all exact-head checks are green.",
  "observed_branch": "feature/ar-1835-authenticated-control-live-isolation-repair",
  "observed_dirty": 0,
  "observed_head": "06565d3de95aa72fa67bf2fde73078753acfd49b",
  "owner": "",
  "plan": "../plans/AR-1835-authenticated-control-live-isolation-repair.md",
  "priority": "P0",
  "schema_version": 1,
  "spec_acceptance": {
    "evidence_class": "contract-test",
    "evidence_digest": "",
    "evidence_ref": "",
    "spec_ref": "specs/AR-1835.json",
    "spec_revision": 1,
    "status": "pending"
  },
  "spec_ref": "specs/AR-1835.json",
  "spec_revision": 1,
  "status": "open",
  "summary": "Repair the missing authenticated ControlClient/server and delegated live-isolation coverage seam without weakening AR-1783 gates.",
  "task_revision": 62,
  "title": "Authenticated control and live-isolation coverage repair",
  "updated_at": "2026-10-13T00:43:07+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1835-authenticated-control-live-isolation-repair"
}
---

Add a bounded test-only authenticated control fixture and/or delegated-isolation runner for the unresolved AR-1783 coverage seam. Keep all authority, provider, credential, network, privacy and quality gates fail-closed and unchanged.

- 2026-10-12T23:49:18+00:00: Dependencies AR-1763, AR-1764 and AR-1782 are complete; scope is
  isolated to the unresolved AR-1783 authenticated control/live-isolation seam.

- 2026-10-12T23:49:39+00:00: Claimed by ar1835-control-isolation-luna.

- 2026-10-12T23:49:47+00:00: Recorded command exit 0; command argv SHA-256
  1d827be58e9b8c543b821a072b6c7c553aea9b697c6c3dac9d5d940f333a8563.

- 2026-10-12T23:50:20+00:00: Recorded command exit 0; command argv SHA-256
  a718b589aef86f256df386ea5f00e348cea5380394519a60fbe919b6a4c614de.

- 2026-10-12T23:50:48+00:00: Recorded command exit 0; command argv SHA-256
  aa8ea94cf916e4d5ac58f248db951862baa070057e971a56386818db167194b8.

- 2026-10-12T23:51:08+00:00: Recorded command exit 0; command argv SHA-256
  7418a9cd16d10da537ddf8c68937bf5cfbffdb99036fee19c6ab593d38cef8d2.

- 2026-10-12T23:51:27+00:00: Recorded command exit 0; command argv SHA-256
  d9fa5fa9d6b1edd0744be1b0aa7351347516f8475fbf10364d8f42e7873456ab.

- 2026-10-12T23:52:10+00:00: Recorded command exit 1; command argv SHA-256
  4a3e600b1837ade382262be90af97cd35a6bf25d60fe5a817c026f468b52cc64.

- 2026-10-12T23:52:38+00:00: Recorded command exit 0; command argv SHA-256
  cb40a50f41779b22fc1031995dcfffcac0b0ec6324166ca5adaecb800c91b826.

- 2026-10-12T23:53:07+00:00: Heartbeat by ar1835-control-isolation-luna.

- 2026-10-12T23:53:21+00:00: Implementation started in the isolated product worktree. Baseline
  asb-control suite passed 95 unit, 32 control, 7 endpoint, 5 schema and 2 doc tests;
  runtime-control bootstrap focused suite passed 2/2. Existing runtime tests are direct fixtures;
  next seam is a real ControlServer/ControlClient runtime bootstrap exchange adapted from the
  established wire fixture. No product commit yet.

- 2026-10-12T23:53:30+00:00: Recorded command exit 0; command argv SHA-256
  35c718f3137dde4f731cdf7e6673ef7c95f80d2f5c55002d935095e8916fd596.

- 2026-10-12T23:54:41+00:00: Code-level seam audit complete. Existing asb-cli control.rs already has
  a real ControlServer/ControlClient transport fixture for plan/launch/reconnect, while
  runtime-control bootstrap tests in lib.rs construct validated responses directly. A safe successor
  fixture must seed RunnerBackend catalog auth and runtime_authorities through its existing
  authenticated setup path, then issue RuntimeBootstrap and RuntimeBootstrapCancel over the real
  Unix socket; copying direct response construction or fabricating authority would violate AR-1835.
  No speculative product change was made. Focused baseline remains green.

- 2026-10-12T23:55:54+00:00: No product implementation commit is safe yet. Exact API seam:
  LiveProviderRuntimeBridge::request_control_bootstrap requires
  ControlClient::session_identity_sha256 to equal the request control_session_sha256 before issuing
  RuntimeBootstrap, while the real RunnerBackend response requires pre-seeded catalog auth and
  runtime_authorities; direct runtime fixtures bypass both. Existing control.rs Unix-socket fixture
  proves transport but only exercises plan/launch/reconnect. Next code action: expose a test-only
  authenticated RunnerBackend seeding constructor or fixture module in asb-cli/asb-control, then add
  an asb-runtime transport test using the real socket for bootstrap, cancel, stale generation and
  replay. Do not fabricate certificate authority or weaken validation. Baseline focused suites
  remain green; no product head was created.

- 2026-10-12T23:55:59+00:00: Checkpointed source commit 5cafc95ec5acdebf7c323e21597666ac0798dfdc.

- 2026-10-12T23:57:17+00:00: Recorded command exit 101; command argv SHA-256
  26d93d2ede288ab44350cfe476efa2cb95e872e072e80e8d7d11a3572ba83ef8.

- 2026-10-12T23:57:34+00:00: Recorded command exit 101; command argv SHA-256
  26d93d2ede288ab44350cfe476efa2cb95e872e072e80e8d7d11a3572ba83ef8.

- 2026-10-12T23:58:00+00:00: Recorded command exit 101; command argv SHA-256
  26d93d2ede288ab44350cfe476efa2cb95e872e072e80e8d7d11a3572ba83ef8.

- 2026-10-12T23:58:33+00:00: Recorded command exit 0; command argv SHA-256
  283d4d2f56a814cb45044388d877bdade81aca1bcdcf0a32a6d7fab4b3079a50.

- 2026-10-12T23:58:55+00:00: Recorded command exit 101; command argv SHA-256
  b0ebf9cd06d13c5170ebbd3bff43836fcd8751ce32d67a2772a56e5b3cc03e2c.

- 2026-10-12T23:59:21+00:00: Recorded command exit 101; command argv SHA-256
  b0ebf9cd06d13c5170ebbd3bff43836fcd8751ce32d67a2772a56e5b3cc03e2c.

- 2026-10-12T23:59:44+00:00: Recorded command exit 101; command argv SHA-256
  b0ebf9cd06d13c5170ebbd3bff43836fcd8751ce32d67a2772a56e5b3cc03e2c.

- 2026-10-13T00:00:21+00:00: Recorded command exit 101; command argv SHA-256
  b0ebf9cd06d13c5170ebbd3bff43836fcd8751ce32d67a2772a56e5b3cc03e2c.

- 2026-10-13T00:00:45+00:00: Recorded command exit 101; command argv SHA-256
  b0ebf9cd06d13c5170ebbd3bff43836fcd8751ce32d67a2772a56e5b3cc03e2c.

- 2026-10-13T00:01:10+00:00: Recorded command exit 101; command argv SHA-256
  b0ebf9cd06d13c5170ebbd3bff43836fcd8751ce32d67a2772a56e5b3cc03e2c.

- 2026-10-13T00:01:34+00:00: Recorded command exit 101; command argv SHA-256
  b0ebf9cd06d13c5170ebbd3bff43836fcd8751ce32d67a2772a56e5b3cc03e2c.

- 2026-10-13T00:01:57+00:00: Recorded command exit 0; command argv SHA-256
  b0ebf9cd06d13c5170ebbd3bff43836fcd8751ce32d67a2772a56e5b3cc03e2c.

- 2026-10-13T00:02:20+00:00: Recorded command exit 1; command argv SHA-256
  f7e20666638201d8567703a3d7bc028e04a5e8e4c6c509de13d4976b0c91a669.

- 2026-10-13T00:02:25+00:00: Recorded command exit 0; command argv SHA-256
  cfddfc1b8766f37e6aede838c174f3c1396c0e28ea1eae79d472cf1b97a74f13.

- 2026-10-13T00:02:48+00:00: Recorded command exit 0; command argv SHA-256
  73f44e0d33813ae3fdbc224a193eafc7788c09dd89d88b73b5e7b68e5b848af8.

- 2026-10-13T00:03:56+00:00: Recorded command exit 101; command argv SHA-256
  534f8613e1c316db8f0f46dd989e44e14a8ca74304a94791b4e4a1c59b2fbb1d.

- 2026-10-13T00:04:48+00:00: Recorded command exit 0; command argv SHA-256
  1c620b5bbc7aabf591dbec959c4f4305405818f87d1405d39c4ff4b7d28f831e.

- 2026-10-13T00:05:09+00:00: Recorded command exit 0; command argv SHA-256
  5733d17d695866e48ffe43b39d66235533702f150bd1be31df6453ae6f04fda3.

- 2026-10-13T00:05:34+00:00: Recorded command exit 0; command argv SHA-256
  1e7fcd0e7a2ec56e7226ecf3ab3c8549369e5485853619d5d074261086d9a260.

- 2026-10-13T00:06:06+00:00: Checkpointed source commit 06565d3de95aa72fa67bf2fde73078753acfd49b.

- 2026-10-13T00:06:27+00:00: Recorded command exit 0; command argv SHA-256
  4028932557964b367f18de88f6007899d9951e1eed7b6217eb7406de0c110a4d.

- 2026-10-13T00:07:45+00:00: Recorded command exit 0; command argv SHA-256
  66861f363dbbdf60adcb8c6f4a4052e71fbbf65b5f1711fc9bcb1bf4767c95e2.

- 2026-10-13T00:08:09+00:00: Recorded command exit 0; command argv SHA-256
  eaead05ed317219b0787fb90a3a1d025631103eeea1b96037096c57d49c217a7.

- 2026-10-13T00:08:31+00:00: Published PR #627 at exact head
  06565d3de95aa72fa67bf2fde73078753acfd49b (tree verified clean; SSH signature and matching DCO
  verified). Independent diff review covered control.rs authenticated seed/socket flow, catalog
  mutation projection validation, and generated diagnostic call-site refresh. Focused control suite
  passed 74 tests; full cargo test --locked --workspace passed product tests (documented ignored
  native/provider lanes); diagnostic_contract ordinary_cli_producers_are_mechanically_closed passed.
  Required exact-head CI is running; do not merge until all required checks are terminal green.

- 2026-10-13T00:09:15+00:00: Publication remains active at PR #627 exact head
  06565d3de95aa72fa67bf2fde73078753acfd49b. Hosted rollup currently has AWQ shadow, Huawei/MIT
  headers, Portable protected-main provenance, and retained faults green; remaining required Rust,
  policy/coverage, credential-free, platform/QEMU, fuzz, Loom/Kani, mutation, and TUI checks are
  still pending. Merge is intentionally blocked on exact-head terminal green checks.

- 2026-10-13T00:10:00+00:00: PR #627 exact head 06565d3 terminal progress: Platform evidence, Kani
  bounded proofs, and Loom/state models are now green; prior AWQ shadow, Huawei/MIT headers,
  protected-main provenance, and retained faults remain green. Bounded fuzz, credential-free
  benchmark, emulated AArch64, TUI journey, mutation sentinels, policy/coverage/supply chain, and
  Rust checks remain pending. Merge remains blocked.

- 2026-10-13T00:10:34+00:00: PR #627 exact-head progress: Bounded fuzz regressions also terminal
  green. Remaining pending required checks are credential-free benchmark, emulated AArch64, exact
  TUI journey, mutation sentinels, policy/coverage/supply chain, and Rust checks. All other listed
  checks remain green; merge stays blocked.

- 2026-10-13T00:11:39+00:00: Exact-head PR #627 remains at 06565d3de95aa72fa67bf2fde73078753acfd49b.
  Hosted TUI job 115063830052 failed in
  pinned_tui_inherited_fd_development_launch_uses_pty_and_cleans_up: terminal application failed;
  crates/asb-cli/src/tui.rs:6127 reported Run Control did not dispatch offline default after
  readiness, then test unwrap at tui.rs:6182. This PR changes only crates/asb-cli/src/control.rs and
  generated diagnostic catalog; failure is outside touched paths and is not yet classified as a
  product regression. Other required checks: Credential-free, bounded fuzz, AWQ, Huawei, Kani,
  Platform, retained faults, Loom, provenance, Matcher/SLO green; Rust, Policy/supply-chain, and
  Emulated aarch64 still pending.

- 2026-10-13T00:12:18+00:00: Recorded command exit 101; command argv SHA-256
  0787225601e8aca9b2229be6659591ce490398e2e03e2b357a2e450a508f0db9.

- 2026-10-13T00:12:23+00:00: Recorded command exit 0; command argv SHA-256
  ff19a6ea039b35afc0cfb1b21f39880f9f055eaac19e4a8655871a6afcf7c913.

- 2026-10-13T00:12:42+00:00: Recorded command exit 0; command argv SHA-256
  13ff74222e991a8047bfbb67bdab6d80394b725632bbfc807f94592afd858b58.

- 2026-10-13T00:13:17+00:00: Recorded command exit 101; command argv SHA-256
  828ca7ac11878ae9ef7065fdd2300b8b21960c377eee33880745278fba0f5046.

- 2026-10-13T00:13:43+00:00: Recorded command exit 0; command argv SHA-256
  828ca7ac11878ae9ef7065fdd2300b8b21960c377eee33880745278fba0f5046.

- 2026-10-13T00:14:09+00:00: Exact TUI failure triage complete. Hosted job 115063830052 failed only
  at tui.rs:6127 with offline-default dispatch readiness, then unwrap at tui.rs:6182. Reproduced
  locally through handoffctl using cross-repo-qualification with ASB_TUI_BINARY plus source
  commit/tree metadata: exact test passed (1 passed, 457 filtered, 5.15s). Initial local attempt
  without required source metadata failed at tui.rs:6014 NotPresent, confirming harness requires all
  workflow env. No AR-1835 code path overlap (diff is control.rs and generated diagnostic catalog);
  classify hosted failure as non-reproducible external/asb-tui qualification flake, not a repair
  candidate.

- 2026-10-13T00:14:15+00:00: Recorded command exit 0; command argv SHA-256
  a92b74ce922367746216f26dbd94db171db9c0daeedfe6f0e0fc4e127a9c72b4.

- 2026-10-13T00:15:26+00:00: Reran failed hosted TUI job through handoffctl: original job
  115063830052 rerun spawned job 115065450191, currently pending. Exact-head PR remains
  06565d3de95aa72fa67bf2fde73078753acfd49b. Required checks still pending: rerun TUI, Rust,
  Policy/coverage/supply-chain, Emulated aarch64; all other required checks remain green.

- 2026-10-13T00:16:42+00:00: Exact-head CI update: Rust checks, Policy/coverage/supply-chain,
  Emulated aarch64, Credential-free, AWQ, Huawei, Kani, Platform, retained faults, Loom, provenance,
  bounded fuzz, and Matcher/SLO all pass. Only rerun Exact TUI inherited-fd and PTY journey job
  115065450191 remains pending; no merge attempted.

- 2026-10-13T00:17:30+00:00: Rerun job 115065450191 also failed with the identical tui.rs:6127
  offline-default dispatch readiness panic and tui.rs:6182 unwrap; all other required exact-head
  checks are green. Local exact workflow reproduction with complete ASB_TUI_* metadata passed. PR
  diff has no tui.rs changes and only control.rs/catalog changes; no signed repair commit is
  justified by evidence.

- 2026-10-13T00:20:47+00:00: Recorded command exit 0; command argv SHA-256
  4028932557964b367f18de88f6007899d9951e1eed7b6217eb7406de0c110a4d.

- 2026-10-13T00:22:01+00:00: Recorded command exit 0; command argv SHA-256
  89191462d5997d17a21c3d4cbf386c3fac9b1533154bb21c28f04f496fb2684f.

- 2026-10-13T00:22:10+00:00: Recorded command exit 0; command argv SHA-256
  139f7a2895297358db6bfc8109b3c825112244ba04fd8f4c7706d45a62d1eba3.

- 2026-10-13T00:39:29+00:00: Recorded command exit 0; command argv SHA-256
  c6bc54c5161c7f24ace82c044ed9826e146343a26111c97d8748b2eac7e2759f.

- 2026-10-13T00:43:07+00:00: Coordinator recovery: no matching worker process is live; PR #627
  required TUI check was rerun against current asb-tui main 486d247 and failed identically at
  tui.rs:6127. Preserve exact logs and external blocker for replacement review.
