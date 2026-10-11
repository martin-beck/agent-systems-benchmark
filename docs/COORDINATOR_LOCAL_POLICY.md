# State-repository coordinator policy

Development review identity is defined in
[`DEVELOPMENT_REVIEW_POLICY.md`](DEVELOPMENT_REVIEW_POLICY.md). An independent
technical review-worker remains mandatory, but a same-account GitHub approval
is sufficient for development integration after exact-head checks, signatures,
DCO, privacy, and post-merge assurance requirements pass. No separate
authorized-maintainer or second-account approval is required. Verified release
and provenance gates remain unchanged.

This file contains project-local policy; `docs/agent-workflow-coordinator.md` remains an
unaltered vendored coordinator release document.

Before merging, require tests, privacy checks, SSH signature/DCO validation and independent
review. The protected integration lane must validate the *merge-result tree* against current
`main`; a green PR is not complete until its immutable merge commit is identified.

Concurrent protected-main movement is normal operational progress, not a worker blocker. A
worker whose topic becomes stale must checkpoint its evidence and remain `in_progress`; it must
not mark its AR or parent goal `blocked` merely because another worker merged. Rebase and repeat
only the checks invalidated by a material change to the reviewed product tree. Generated
coordinator projections, live worktree observations, and unrelated task records do not by
themselves invalidate independent review, source signatures, DCO, privacy evidence, or product
tests. The integration lane must revalidate the resulting merge tree and run exact-main assurance
before publication.

Only an unavailable required external dependency, a failed safety gate without an available
repair, or an explicit human decision may justify `blocked`. Integration contention instead uses
retry/backoff, durable receipts, and a serialized admission queue. Do not use the outer agent
goal status as a substitute for an AR lifecycle transition.

AR-1796 tracks the required enforceable queue, receipt-reuse, and liveness implementation. Until
then, coordinators must batch state projection reconciliation and avoid committing high-frequency
live worktree observations to protected `main` during an active integration cycle.

Immediately after merging, watch every required post-merge assurance workflow for the exact
resulting main SHA until each reaches a terminal result. Record workflow names, run IDs,
conclusions, and failures or cancellations in the owning task; do not classify a merge as
released or usable while post-merge assurance is pending or failed.

Release to `done`, `open`, or `blocked` with the merge and post-merge evidence; reconcile and
run the live doctor.
