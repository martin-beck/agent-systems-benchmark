# AR-1454 protected-main tree-equality repair

This record preserves the failed PR #328 admission as immutable evidence. The
protected-main Repository Quality workflow `36188339692` rejected merge
`452f3ca29390ab37cf3aff8c813b92b54b163b20` because the reviewed topic tree was
not the exact tree produced by merging the then-current protected base. The
failure is not waived and protected history is not rewritten.

The repair branch is based on the current protected `main` descendant and
contains only this evidence record. Admission remains subject to the existing
protected-main policy: a signed+DCO topic commit, exact-head checks, an
independent review, a two-parent non-squash merge, reviewed-topic tree equality,
exact deterministic merge-preview equality, and all seven terminal-success
post-merge workflows.

This document is evidence of the repair scope only. It does not alter policy,
required checks, merge topology, or release gates.
