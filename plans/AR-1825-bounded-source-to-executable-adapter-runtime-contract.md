# AR-1825 plan: bounded source-to-executable adapter/runtime contract

1. Freeze the source-to-executable contract against AR-1776's catalog and
   AR-1778's pinned builder: immutable source revisions, explicit strategy
   identifiers, bounded project-local dependency/runtime closure, and no
   runtime acquisition or network access.
2. Define and implement a per-tool support matrix. Each row names the adapter,
   package/install strategy, source identity, launcher/entrypoint identity,
   regular-file/symlink and mode policy, runtime closure, offline fixture, and
   supported or typed-unsupported result.
3. Materialize through adapter-owned bounded argument arrays and transactional
   staging. Verify source/archive identity, output type, symlink target, mode,
   launcher identity, runtime compatibility, and bounded smoke execution before
   activation; reject unknown strategies and archive outputs without adapters.
4. Add content-minimized receipts binding source revision, adapter/strategy,
   dependency/runtime closure, executable and launcher digests, smoke result,
   rollback target, and retention decision. Preserve the last known-good state
   when a new attempt fails and clean only transaction-owned staging data.
5. Add deterministic offline positive fixtures and hostile negative fixtures for
   unknown strategies, missing/substituted entrypoints, unsafe symlinks or modes,
   runtime mismatch, unbounded installers, runtime downloads, and non-executable
   archive output. Prove no process or usable-state effect occurs on rejection.
6. Run the focused and full ASB gates, obtain independent review, exact-head CI,
   signed-DCO protected merge, exact-main qualification, and acceptance evidence;
   only then update AR-1779's successor link and pending next action as allowed.
