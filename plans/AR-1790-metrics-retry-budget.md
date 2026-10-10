# AR-1790 plan: repair the effective metrics retry budget

1. Read the complete ASB development documents and AR-1790 spec/plan/task,
   plus the AR-1789 independent review receipt.
2. Inspect the exact retry guard and design a deterministic test seam for
   transient `WouldBlock` spawn failures; preserve bounded deadlines and typed
   exhaustion.
3. Repair the guard and add positive coverage beyond the old third attempt plus
   negative coverage for exhaustion. Do not use unbounded retry or serial-only
   workspace execution.
4. Run focused metrics/control/rustup tests and repeated ordinary workspace
   gates, then fmt, clippy, rustdoc, privacy, and diff checks via handoffctl.
5. Obtain independent exact-head review and release the repaired candidate for
   AR-1789 re-review. Merge only through the signed exact-tree procedure.

This development repair does not require a GitHub Reviews API approval object.
