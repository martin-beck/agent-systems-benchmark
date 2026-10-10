# AR-1787 plan: correct control lock lifetime and catalog identities

## Scope

Repair the P0 flaw found while reviewing AR-1786. Its POSIX record-lock guard
must preserve the active owner when a same-process contender is rejected: closing
any descriptor for the inode must never release the active lock. The repair must
also update the line-addressed diagnostic catalog deterministically.

## Required work

1. Read the complete ASB development, architecture, and quality documentation and
   inspect AR-1786's exact blocked review receipt.
2. Reproduce the same-process contender/drop failure and the stale diagnostic
   identities from a clean product-bound worktree.
3. Design an ownership-safe descriptor lifecycle. A rejected contender must not
   close a descriptor in a way that releases the active process lock; accepted
   ownership must remain explicitly guarded, and fork/exec descendants must not
   retain it.
4. Add focused positive/negative tests for active-owner preservation, rejected
   contenders, child inheritance, cleanup, and bounded concurrency. Update only
   the reviewed catalog identities/causes required by the source changes.
5. Run repeated ordinary `cargo test --locked --workspace`, diagnostic closure,
   fmt, Clippy, rustdoc, privacy/diff checks, independent review, hosted CI,
   signed protected merge, and exact-main workflows.

## Non-goals

Do not use retries, test exclusion, serial-only execution, unsafe unlocks,
global mutable authority bypasses, or a GitHub Reviews API approval requirement.
