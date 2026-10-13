# AR-1837 plan: PR #627 protected-main identity recovery

1. Preserve invalid web merge `6ea3648e483f25179c3112b54ed7c348e141cad4`, its
   exact failure, and the current protected-main tip; never rewrite history.
2. Read the complete development and merge-integrity documentation. Create a
   clean isolated product worktree at current main and make the smallest
   metadata-only signed-DCO recovery topic.
3. Run repository policy, signature, DCO, privacy, focused and full applicable
   gates; obtain independent exact-head review and terminal-success CI.
4. Publish the exact two-parent signed merge only through
   `tools/integration/merge_pr.py --push`, using the established local identity
   and exact target lease.
5. Verify portable provenance and all exact-main workflows, record the durable
   receipt, release AR-1837, and reconcile AR-1835's post-merge evidence.
