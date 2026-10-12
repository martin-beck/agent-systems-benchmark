# AR-1821 implementation plan

1. Reconcile current state, inspect PR #128, and verify its exact accepted head
   and required checks against current `main`.
2. Claim and bind this recovery AR, then use the established signed local
   integration helper to construct and publish the two-parent merge. Never
   force-push or merge through GitHub Web Flow.
3. Run exact-main post-merge state schema, generated-view, vendor, and doctor
   checks; preserve any transient runner failure as evidence and rerun only
   through the normal gates.
4. Record a concise durable receipt and release AR-1821 done only when all
   acceptance predicates pass.
