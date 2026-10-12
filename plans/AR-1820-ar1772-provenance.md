# AR-1820 plan: AR-1772 protected-main provenance and DCO requalification

1. Preserve PR #602, merge 4b14797a0892988a484a5171c45c4e8ec3c3d836, and
   failed runs 38210171658, 38210171401, and 38210171442 as immutable evidence.
   Record parents, reviewed/topic tree, merge tree, signature, DCO, and the
   exact failure text.
2. Refresh the state repository from current origin/main and create a dedicated
   non-overlapping worktree. Reproduce the protected-main policy result locally
   against the immutable merge and current policy; do not rewrite history or
   weaken any gate.
3. Use the documented local signed-integration path to produce only a
   scope-authorized recovery descendant, preserving product tree and all
   historical evidence. If implementation changes are proven necessary, stop
   and open a separately scoped product repair rather than altering this AR.
4. Independently review the exact candidate, run focused and full applicable
   state/product gates, verify SSH signatures, DCO, privacy, topology, tree
   identity, and hosted exact-head results.
5. Publish only from a clean exact tree, wait for terminal exact-head CI, merge
   through the approved local integration path, verify every post-merge gate,
   and record a durable receipt before releasing this AR. Keep AR-1773 and
   AR-1775 unreleased until this closes.
