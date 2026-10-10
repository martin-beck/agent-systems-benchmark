# AR-1786 plan: repair control state-root ownership races

## Scope

Repair the reproducible `control state root is already owned` collision exposed by
the independent AR-1785 review. The ordinary locked workspace test command must be
deterministic; serializing the gate or excluding tests is not acceptable.

## Required work

1. Read the complete ASB development, architecture, and quality documentation and
   inspect AR-1785's exact review receipt and failing test identities.
2. Reproduce the failure from a clean product-bound worktree using the ordinary
   `cargo test --locked --workspace` command and bounded repeated runs.
3. Trace control state-root allocation, ownership markers, cleanup, and test
   parallelism. Implement per-test isolation or an equivalent ownership-safe
   lifecycle without weakening runtime authority checks.
4. Add focused positive, collision, cleanup, and failure-recovery tests. Preserve
   all existing control lifecycle semantics and privacy boundaries.
5. Run focused tests, repeated ordinary full workspace tests, fmt, clippy, rustdoc,
   exact-head review, hosted CI, signed protected merge, and exact-main workflows.
6. Re-review AR-1785 at its repaired exact head only after this gate is green.

## Non-goals

Do not skip tests, force serial execution as the only fix, weaken ownership checks,
change provider behavior, require a GitHub Reviews API approval object, or publish
credentials/private paths.
