# AR-1789 plan: repair metrics fixture nondeterminism

1. Read the complete ASB development documents and the AR-1789 spec/plan/task,
   plus the AR-1788 review receipt that identified the failure.
2. Reproduce `kernel::tests::missing_malformed_and_unsafe_configuration_fail_closed`
   in bounded ordinary workspace runs and in isolation; capture the exact
   assertion and any shared environment/configuration root.
3. Repair fixture isolation, cleanup, or synchronization while preserving the
   typed malformed/unsafe rejection contract; do not exclude the test or make
   the workspace serial-only.
4. Add positive and negative regression tests and update generated diagnostics
   if source identities move.
5. Run focused tests, repeated full workspace gates, fmt, clippy, rustdoc,
   privacy, and diff checks through `tools/handoffctl run`.
6. Obtain independent exact-head review, exact-head CI, signed DCO integration,
   and exact-main post-merge verification before releasing this AR.

This development repair does not require a GitHub Reviews API approval object.
