---
{
  "branch": "feature/ar-1835-authenticated-control-live-isolation-repair",
  "checkpoint_commit": "5cafc95ec5acdebf7c323e21597666ac0798dfdc",
  "claim_expires": "2026-10-13T01:53:07+00:00",
  "depends_on": [
    "AR-1763",
    "AR-1764",
    "AR-1782"
  ],
  "id": "AR-1835",
  "next_action": "Implement the test-only authenticated backend seeding seam first; then add real Unix-socket runtime bootstrap/cancel positive and stale/replay tests and rerun focused/full gates.",
  "observed_branch": "feature/ar-1835-authenticated-control-live-isolation-repair",
  "observed_dirty": 1,
  "observed_head": "da0cbe0848a10e61a2cd0b9a3144217842a3369b",
  "owner": "ar1835-control-isolation-luna",
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
  "status": "in_progress",
  "summary": "Repair the missing authenticated ControlClient/server and delegated live-isolation coverage seam without weakening AR-1783 gates.",
  "task_revision": 25,
  "title": "Authenticated control and live-isolation coverage repair",
  "updated_at": "2026-10-12T23:59:44+00:00",
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
