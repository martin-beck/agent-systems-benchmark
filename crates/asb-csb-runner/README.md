# ASB CSB execution boundary

`asb-csb-runner` is the opt-in, versioned boundary between ASB and one
content-pinned CSB executable. It does not link CSB, Python, benchmark
generators, or syzkaller into the default ASB runtime.

The boundary accepts only the `external_application` execution mode, relative
artifact paths, an allowlisted environment, bounded arguments, and SHA-256
identities for the executable and source tree. Negotiation uses the existing
ASB JSON-RPC v1 framing contract. Shell commands, network access, ambient
credentials, plugins, generators, and syzkaller coupling are rejected.

Execution is not supported until the caller has independently verified the
pinned executable bytes and supplied AR-0103 sandbox and resource-lease
containment. A durable `Running` intent must precede spawn. If spawn or cleanup
cannot be proved, the caller must retain `NeedsReconciliation`; it must not
retry an uncertain effect.
