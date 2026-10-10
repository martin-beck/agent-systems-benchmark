# AR-1788 plan: repair rustup fixture nondeterminism

1. Read the complete AR-1787 task, review receipt, and ASB development documents.
2. Reproduce the reported full-workspace failure using bounded repeated ordinary
   `cargo test --locked --workspace` runs and inspect only the named fixture and
   its test dependencies.
3. Repair the fixture's temporary roots, environment cleanup, process lifetime,
   or synchronization without excluding tests or making the workspace serial-only.
4. Add positive and negative regression tests and update generated diagnostic
   identities if source lines move.
5. Run focused tests, repeated full workspace gates, fmt, clippy, rustdoc,
   privacy, and diff checks; record all failures and next actions through
   `tools/handoffctl run`.
6. Obtain independent exact-head review, exact-head CI, signed DCO integration,
   and exact-main post-merge verification before releasing this AR.

This development repair does not require a GitHub Reviews API approval object.
Do not weaken rustup validation, suppress the failing test, or claim a single
passing rerun proves determinism.
