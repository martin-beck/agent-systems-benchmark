# Development-host workflow routing

ASB uses disposable GitHub-hosted Ubuntu runners for every `pull_request` job
and every required public contribution gate. Persistent development-host
capacity is selected only by the indivisible label
`asb-development-v1-x86_64-ubuntu2404` in two protected manual workflows.

| Workflow | Event | Trust class | Checkout | Purpose |
| --- | --- | --- | --- | --- |
| Rust, quality, formal, fault assurance | push, pull request, manual | disposable | event revision | Required public gates |
| Development host runner canary | manual on `main` | trusted qualification | none | Prove exact-label routing and disposable temporary state |
| Trusted development host validation | manual on `main` | trusted protected revision | exact `github.sha` without persisted credentials | Exercise runner lifecycle controls |

The trusted workflow has no inputs, rejects any repository or ref other than
the public ASB `main` branch, uses read-only contents permission, and pins its
sole action by full commit. A fork cannot dispatch an upstream repository
workflow. Pull requests never select the development-host label, and the
repository policy rejects adding an automatic trigger, widening permissions,
removing the trusted repository/ref guard, using a partial label, or adding
another persistent-runner workflow.

An offline runner leaves the manually dispatched job queued; it never falls
back to a generic or hosted label. Label or architecture drift likewise fails
closed. The external runner lifecycle must pass its private lease and
installation health checks before starting. Job output reports only the public
trust class, label-selected OS/architecture result, and pass/fail status; it
must not print a runner name, host identity, runtime root, token, environment,
or raw diagnostic log.

AR-0831 defines routing but does not activate ordinary push or pull-request
jobs on this capacity. AR-0832 must qualify repeated clean runs, reset,
isolation, and recovery before any broader activation. AR-0833 separately owns
tokenless post-boot ephemeral registration; reboot persistence remains
unsupported until that work is complete.
