# Native `asb easy` lifecycle

The native guided lifecycle is provider-free and requires no Cargo, Make, Just,
Python, or shell prerequisite for users of the `asb` binary. The development
qualification channels are `dev` and `stable`; `stable` is deliberately a
local development mock and is never public-release or stable-promotion
evidence. `nightly` and `experimental` are rejected until their manifests are
available.

```text
asb easy build --channel stable
asb easy install --channel stable --yes
asb easy status --json
asb easy test --channel stable
asb easy update --yes              # preserves the active channel
asb easy rollback --yes
asb easy remove --yes
```

Mutating operations require `--yes`; `--dry-run` validates the channel and
prints the bounded, versioned result without changing state. Every lifecycle
result identifies `channel_kind: development_mock`, `mock: true`, and
`public_release_evidence: false`. `--json` is accepted in any option position
and emits the same versioned envelope used by the rest of the CLI. Lifecycle
tests perform only local state checks and do not contact a provider.

An omitted channel on `update` retains the active channel. Invalid or
unavailable channels fail before state mutation. Rollback uses the prior
content-addressed channel recorded by the local lifecycle state; repeated
rollback without a target is a typed validation error.
