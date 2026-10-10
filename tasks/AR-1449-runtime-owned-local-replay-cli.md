---
{
  "branch": "codex/ar-1449-current-main",
  "checkpoint_commit": "",
  "claim_expires": "2026-10-11T02:04:16+00:00",
  "depends_on": [
    "AR-1443",
    "AR-1448"
  ],
  "id": "AR-1449",
  "next_action": "Requalify the current protected-main runtime-owned replay provisioner against generated cassettes; repair any strict offline authority failure, then independently review and run exact-head CI.",
  "observed_branch": "codex/ar-1449-current-main",
  "observed_dirty": 0,
  "observed_head": "59cfd9cf6f068ff8945eb093a04f0bbb0c7bb7c1",
  "owner": "codex-ar1449-runtime-replay-20261011",
  "plan": "../plans/AR-1449.md",
  "priority": "P1",
  "schema_version": 1,
  "status": "in_progress",
  "summary": "Provide fail-closed runtime-owned local replay authority for the guided CLI wrapper.",
  "task_revision": 23,
  "title": "Runtime-owned local replay CLI authority",
  "updated_at": "2026-10-10T23:04:16+00:00",
  "worktree_key": "agent-systems-benchmark-ar-1449-current-main"
}
---
AR-1338 exposes the guided replay route, but the normal argument-only CLI intentionally receives no ReplayLaunchAuthority. This successor must provide a runtime-owned local deterministic replay acquisition boundary using the existing AR-1448 source and AR-1443 replay contracts.

Requirements:
- derive cassette identity from the validated cassette before any authority issuance;
- acquire bounded private relay, benchmark lease, network-denied sandbox input and runtime attestation inside the runtime boundary;
- inject one-shot authority into the ordinary CLI without accepting caller-built authority or secret bytes;
- preserve strict replay, cancellation, teardown, path confinement, offline operation and direct or alternate egress denial;
- provide positive and hostile tests for stale, copied, mismatched and unavailable authority;
- use local deterministic fixtures or LiteLLM-compatible mocks only; external providers and native hosts are not required.

Do not weaken AR-1448 or make live provider reachability a prerequisite.

- 2026-09-25T16:55:07+00:00: Claimed by ar1449-replay-luna56.

- 2026-09-25T16:56:36+00:00: Independent audit: AR-1448 ReplayAuthoritySource::issue requires
  caller-built SandboxLaunchInput, benchmark ResourceLease, SandboxBackend and relay attestation;
  ordinary CLI cannot safely acquire these. Promote a runtime-owned local replay acquisition factory
  successor; no product changes made.

- 2026-10-10T22:58:00+00:00: Dependencies AR-1443 and AR-1448 are complete. Promote runtime-owned
  local replay authority repair to unblock AR-1698; local/mock fixtures are sufficient and no GitHub
  Reviews API approval object is required.

- 2026-10-10T22:59:56+00:00: Claimed by codex-ar1449-runtime-replay-20261011.

- 2026-10-10T23:01:02+00:00: Recorded command exit 1; command argv SHA-256
  ebdb48c305bb702bc79c10e4a2331535c714ba52d210160e798693a9184f5d73.

- 2026-10-10T23:01:13+00:00: Recorded command exit 0; command argv SHA-256
  c89423b254146cbfb809f17acb09745c0d3b147e4b8e52b9bdc18a181394fc84.

- 2026-10-10T23:01:16+00:00: Recorded command exit 0; command argv SHA-256
  2b245f3f633ccd5d4e76b46865b4c38975ea764a922263c5a2cb4b711d4b3167.

- 2026-10-10T23:01:25+00:00: Recorded command exit 0; command argv SHA-256
  9e77938986d4d8c61d283d8de879bbdcfb3764435aa5a0b61dfb57124ce2cf7f.

- 2026-10-10T23:01:35+00:00: Recorded command exit 1; command argv SHA-256
  b7c438686581cf292b6cd41497ceabb3af42dfed57e04dfd6cc89efed8e2d15b.

- 2026-10-10T23:02:14+00:00: Terminal blocker: declared
  feature/ar-1449-runtime-owned-local-replay-cli is an unrelated-history stale branch (HEAD
  f03d9e484d6ca73eacdbd5476980bf33ca737540; origin/main 59cfd9cf6f068ff8945eb093a04f0bbb0c7bb7c1). A
  handoffctl-wrapped rebase onto origin/main stopped at commit 2579362d with hundreds of add/add
  conflicts across the repository; minimal git rebase --abort recovery restored clean declared
  worktree. No product mutation was published. Current protected origin/main already contains
  runtime-owned LocalReplayProvisioner/development_fixture acquisition and guided replay wiring
  (03d98ac7 and descendants), so cherry-picking onto this unrelated branch is unsafe. Requires
  coordinator rebind/rebase successor; preserve AR-1791 unrelated dirty state.

- 2026-10-10T23:03:44+00:00: Claimed by codex-ar1449-rebind.

- 2026-10-10T23:03:47+00:00: Recorded command exit 0; command argv SHA-256
  74dcd1cb28fa2fad4b01a1261e8fa21822af4c19bc8a7c826b6f4129e8f8ca8b.

- 2026-10-10T23:03:54+00:00: Recorded command exit 1; command argv SHA-256
  a1971ccc85a0b7970f0999a449d90195e58b810afb183deea60b803f531af2af.

- 2026-10-10T23:04:02+00:00: Recorded command exit 0; command argv SHA-256
  7a147ab4419597fad6c3452d0c44b3d91672c596be6055fbca205ab54b9c1dbd.

- 2026-10-10T23:04:09+00:00: Rebound stale AR-1449 metadata to current protected-main
  branch/worktree; worker may now requalify the existing runtime-owned replay provisioner.

- 2026-10-10T23:04:16+00:00: Claimed by codex-ar1449-runtime-replay-20261011.
