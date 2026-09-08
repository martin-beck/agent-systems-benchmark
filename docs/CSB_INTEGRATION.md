# CSB orchestration compatibility

AR-0601 compared ASB's direct execution contract with the public CSB source pinned by
[CSB execution provenance](CSB_PROVENANCE.md). The comparison is intentionally limited to CSB
commit `d577c5249501b29e33a87524a677a101477d5579`, tree
`97d08b39026f7c7d3e6f748b26a20e819a92184f`. It does not qualify a later CSB revision or an
ambient Python installation.

## Compatibility decision

CSB's high-level `bm-runner` is not an eligible ASB execution backend. The inspected runner:

- constructs and runs a benchkit `CampaignSuite`, so it owns campaign repetition and scheduling;
- starts its own execution units behind a file barrier, waits for them, and performs cleanup;
- creates, starts, stops, and collects its own monitors and invokes configured plugins;
- constructs application commands as shell strings; and
- imports a floating Python dependency graph before argument processing.

Those behaviors conflict with ASB's requirement that one scheduler and one collector own a run.
They also bypass the versioned, bounded, shell-free AR-0603 boundary. An isolated startup probe
using the system interpreter with user site packages disabled and `-S` fails before argument
processing because `pandas` is unavailable. Installing the dependency would not resolve the
unlocked dependency graph recorded in the provenance audit.

The only compatible surface is `asb-csb-runner`'s `external_application` operation. That surface
keeps scheduling, resource leases, deadlines, cancellation, durable state, and artifact commits
under ASB ownership. It admits the exact pinned standard-library-only
`bm-external/bwrap/bm-bwrap.py` bytes and no CSB monitors or plugins.

## Why an engineering-workload bridge is not yet supported

The qualified CSB script accepts an inner `--command` and launches it with Python
`subprocess.run`. AR-0603 binds the outer script to a verified open descriptor, but its request
does not bind the inner executable to an already-open immutable descriptor. A digest check in a
higher-level adapter would therefore leave a verification-to-execution replacement window. The
script also uses `parse_known_args`, and its semicolon-delimited stdout is neither a versioned
result schema nor a durable artifact contract.

Consequently AR-0601 does not claim that an ASB engineering fixture has run through CSB, that CSB
and direct-ASB artifacts or timestamps are equivalent, or that high-level CSB monitoring is
supported. The already-qualified `/usr/bin/true` probe establishes containment of the outer CSB
fixture only; it is not an engineering workload or a direct-versus-CSB equivalence result.

A future bridge can be considered only after the execution boundary:

1. stages or opens the inner executable and launches those exact verified bytes without a pathname
   re-resolution window;
2. admits a closed argument grammar rather than forwarding arbitrary or ignored options;
3. defines a bounded versioned result and artifact mapping that excludes CSB timing when ASB owns
   timing; and
4. proves, on one immutable engineering fixture, matching direct and CSB-backed termination,
   cancellation, artifact bytes, and ASB-owned timestamps with zero residual descendants or
   leases.

Until then the optional orchestration bridge is unavailable. Normal ASB builds and runs retain no
Python, benchkit, generator, syzkaller, CSB monitor, or plugin dependency.
