# AR-1799 plan: repair the remaining `asb tui install` rustup rejection

1. Refresh ASB and asb-tui references, reconcile coordinator state, and record
   exact protected-main heads. Reproduce the user-visible `trusted_tool_invalid`
   through the installed `asb tui install` path in a disposable HOME.
2. Capture the exact failing validator and distinguish permission/ownership
   findings from hard path-shape, symlink, malformed-settings, unavailable-tool,
   and descriptor/substitution failures.
3. Implement the smallest development-only repair. Keep stable/production
   policy strict, keep development auth/signature/key checks warning-only, and
   do not widen ambient PATH or accept arbitrary tools.
4. Add regression tests that exercise the real install/materialization route
   for conventional user-owned, group/world-writable, and non-user-owned
   fixtures, plus hostile negative cases. Assert human and `--json` diagnostics.
5. Run focused rustup/rustc/TUI tests, exact locked workspace gates, independent
   review, signed/DCO PR integration, and post-merge installed-entrypoint
   qualification before releasing the AR.
