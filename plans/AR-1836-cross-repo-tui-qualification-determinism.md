# AR-1836 cross-repository TUI qualification determinism

AR-1835/PR #627 is blocked by two identical hosted failures in the exact
inherited-fd/PTY qualification while the same test passes locally when all
workflow metadata is supplied. Determine whether the seam is the workflow
harness or the asb-tui development contract, then make the smallest
reviewed deterministic repair without weakening any assertion.

Acceptance requires two fresh hosted exact-head passes, complete immutable
source/binary binding, bounded diagnostics, privacy review, and signed DCO
integration. Do not modify unrelated TUI behavior or unblock AR-1835 by
waiving the check.
