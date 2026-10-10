# AR-1791 plan: repair recurrent TUI fixture contamination

1. Read the ASB development documents and the complete AR-1791 task/spec, then inspect the exact AR-1785 failure evidence.
2. Bind an isolated product worktree at current protected main and identify the GCC descendant-marker and rustup descriptor cleanup races.
3. Repair both fixtures with unique RAII-owned roots, explicit descriptor/child cleanup, and focused positive/negative coverage. Do not weaken the fixture assertions or serialize the workspace gate.
4. Run focused tests repeatedly, then repeated ordinary locked workspace tests, fmt, clippy, rustdoc, privacy, and diff checks through handoffctl.
5. Obtain independent exact-head review and complete signed exact-tree integration and post-merge verification before releasing AR-1791 and unblocking AR-1785.

This development repair does not require a GitHub Reviews API approval object.
