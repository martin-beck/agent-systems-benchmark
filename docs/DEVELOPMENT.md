# Coordinated development

This is the canonical process for both ASB repositories. The sibling
agent-systems-benchmark-state checkout contains tasks, plans and handoffctl.
Use the configured remote development host and second-drive storage for source,
worktrees, toolchains, caches, artifacts and all build/test activity. Host details
belong only in ignored runtime configuration, not public coordination state.

## Start and recover

Read PLAN, ARCHITECTURE and QUALITY and preserve all existing work. Reconcile
handoffctl state, Git refs, processes, CI and worktrees before repeating any action
after an interruption. Reconcile and snapshot from the product root:

```sh
../agent-systems-benchmark-state/tools/handoffctl reconcile --commit --push
../agent-systems-benchmark-state/tools/handoffctl snapshot
```

Read the complete selected AR and plan, dependencies, revision and checkpoint.
Only open tasks with completed dependencies may be claimed. Planned work becomes
open through a reviewed state change when dependencies and resources are ready.
One worker owns one active AR and one named branch/worktree at a time.

```sh
../agent-systems-benchmark-state/tools/handoffctl claim AR-NNNN --owner WORKER_ID --lease-minutes 120
```

Use the coordinator wrapper for product/Git/build/review mutations. For worktree
creation, run the command from the canonical checkout under the claimed task.
Worktree names and feature branches are declared by each AR.

```sh
../agent-systems-benchmark-state/tools/handoffctl run AR-NNNN --owner WORKER_ID -- COMMAND ARGUMENTS
```

Heartbeat at least hourly and before lease expiry. Publish concise evidence after
every material result, failure or changed next action. Never copy prompts, raw
logs, credentials or private machine details into public tasks. Expired ownership
permits investigation; it does not authorize repeating ambiguous external effects.
No forced updates or destructive checkouts to resolve concurrent work.

## Integration

Treat shared schema and Cargo workspace changes as coordinator-owned integration
points. Freeze contracts before parallel adapter work; each worker modifies only
its AR paths. Serialize merges and benchmark-host reservations while allowing
independent source work. A failed required gate preempts further feature work.

Use focused commits with git commit -S -s, Martin Beck's configured gmx.de identity,
and Linux-kernel-style matching Signed-off-by trailers. New product work uses Rust.
Retain and test the reused Python coordinator; do not dilute its fault coverage.

Product changes go through pull requests after the initial repository bootstrap.
Machine task transitions may fast-forward to the state repository main through
handoffctl; schema/tooling/policy changes need review and CI. Do not manually edit
generated CURRENT, PROJECT_STATE or WORKTREES views. Both repositories reject
unrelated private project history and runtime configuration.

A task is done only when its acceptance criteria, applicable exact-head CI and
post-merge evidence pass and the durable state matches the actual result.
Release paused tasks as open with the exact next action, or blocked for a named
external dependency. Never leave stopped workers marked in_progress.

```sh
../agent-systems-benchmark-state/tools/handoffctl release AR-NNNN --owner WORKER_ID --status done --note "Verified outcome and evidence"
../agent-systems-benchmark-state/tools/handoffctl reconcile --commit --push
../agent-systems-benchmark-state/tools/handoffctl doctor --live
```

AR-0001 is the bounded initial bootstrap exception to the claim/wrapper process:
it establishes the two repositories, tool, schema, baseline tests and task graph
before ordinary workers start. Later implementation follows the complete process.
