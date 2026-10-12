# AR-1823 implementation plan

1. Reconcile current state and verify the detached AR-1820 task and receipt
   contents against their recorded merge, checks, digest, and privacy claims.
2. Promote, claim, and bind this cleanup task through `handoffctl`.
3. Publish the exact existing AR-1820 task/receipt evidence on current main;
   do not rewrite historical PRs or add runtime changes.
4. Run exact-main coordination validation and live doctor, then release this
   cleanup AR with durable evidence.
