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

Both persistent-runner workflows have no inputs and reject any repository or
ref other than the public ASB `main` branch. They use read-only contents
permission; the trusted validation pins its sole action by full commit and the
canary runs no action or repository checkout. A fork cannot dispatch an
upstream repository workflow. Pull requests never select the development-host
label, and repository policy rejects adding an automatic trigger, widening
permissions, removing either repository/ref guard, explicitly printing runner
or host identity, using a partial label, or adding another persistent-runner
workflow.

An offline runner leaves the manually dispatched job queued; it never falls
back to a generic or hosted label. Label or architecture drift likewise fails
closed. The external runner lifecycle must pass its private lease and
installation health checks before starting. The canary suppresses path-bearing
utility output and emits only a fixed pass/fail message. Trusted validation
reports public source-test status. Runtime assertions verify the trust class and
label-selected OS/architecture without intentionally printing a runner name,
host identity, runtime root, token, environment, or raw diagnostic log.

GitHub generates self-hosted job metadata outside workflow control. That
metadata can include the pseudonymous runner registration name and the machine
name, so these public workflow logs do **not** prove complete host-identity
confidentiality. Operators must use a non-sensitive pseudonymous registration
name and machine identity. ASB claims only that its checked-in workflow steps
avoid explicit identity output; policy tests cannot sanitize platform-generated
metadata.

AR-0831 defines routing but does not activate ordinary push or pull-request
jobs on this capacity. AR-0832 must qualify repeated clean runs, reset,
isolation, and recovery before any broader activation. AR-0833 separately owns
tokenless post-boot ephemeral registration; reboot persistence remains
unsupported until that work is complete.
