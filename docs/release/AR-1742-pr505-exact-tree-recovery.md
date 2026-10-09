# AR-1742 PR #505 exact-tree recovery

This record preserves the PR #505 publication-integrity failure as immutable
evidence. The reviewed topic `e424c392d7bcd99199ed8f194918656a27d4b65b`
had tree `1aa96736bbbc6d94b8555ebe0da237c3dd7ab7d2` and merge base
`1a5888ce1c96414015bbaf223ac42302871d47fe` with the protected target
`2f7387e5c269449f2337ece3bf702e2a76ee67c3`. Published merge
`a9abcf2e63f761e314593e9abc6bf074b7418e5e` instead had tree
`befcb782d6ce260d1d4dd0e4fe25c2fb0b1b900f`. Repository Quality run
`37758191633` and Rust verification run `37758191874` rejected that exact
reviewed-tree mismatch. The failure is not waived and protected history is not
rewritten.

This forward recovery is based on a current protected `main` descendant and
contains only this evidence record. Admission remains subject to the existing
protected-main policy: a signed and DCO-bearing topic commit, exact-head checks,
independent review, a signed two-parent non-squash merge, equality between the
reviewed topic tree and published merge tree, deterministic merge-preview
equality, and terminal-success exact-main workflows.

AR-1740 follow-up PR #507 is a separate default-lifecycle product change. Its
merge `d53e901677024741cdf0477c9e1efb5d0b664c02` and successful hosted checks
do not replace, waive, or retroactively qualify the PR #505 recovery.

This document records recovery scope and immutable identities only. It does not
alter policy, required checks, merge topology, product support, or release
gates.
