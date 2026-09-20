# Authenticated provider readiness successor

This document records the implementation boundary for the authenticated
provider-readiness producer needed by the setup wizard. It is deliberately a
qualification artifact: the current control backend does not yet have every
authoritative observation required to publish a readiness result.

## Current authoritative observations

The current runner durably owns and exposes:

- selected agent IDs, provider profile, model, authentication method, and a
  credential-reference digest in `ConfigurationSnapshot`;
- provider/model/auth-method availability in `ProviderCatalog`; and
- provider enrollment lifecycle and credential-reference digests in
  `AuthStatusResponse`.

The `asb-agents` crate also contains bounded authenticated request and probe
building blocks. They are not currently wired into the control backend's
durable provider enrollment path. In particular, `AuthEnroll` records a
credential reference and endpoint identity but does not execute a provider
probe, and `ProviderCatalogAction::Refresh` intentionally returns
`capability_unavailable`.

Consequently, `configured=true`, an enrolled credential reference, or an
available static provider catalog entry must not be interpreted as endpoint
reachability or provider authorization. A frontend must not combine these
projections locally to infer readiness.

## Successor implementation seam

After the missing producers are qualified, add one versioned authenticated
control operation whose response is a closed projection of verified facts:

`configuration_present`, `configuration_complete`, `endpoint_available`,
`configuration_malformed`, `configuration_stale`, and `authorized`.

The producer must derive these facts from the validated runner configuration,
verified agent catalog, provider catalog, and an authenticated provider
connectivity result bound to the enrolled credential generation and provider
catalog generation. It must include the generations used for the projection
and reject contradictory combinations. It must never accept readiness,
endpoint, authorization, process, path, or secret values from the frontend.

The consumer precedence is fixed:

`malformed -> unavailable -> unconfigured -> incomplete -> stale ->
unauthorized -> configured`.

Unknown protocol versions, unknown fields, stale generations, missing producer
evidence, unauthorized peers, and oversized responses fail closed. The response
contains only bounded IDs, enum/reason codes, generations, and digests; never
credentials, endpoint values, host paths, prompts, payloads, or raw provider
diagnostics.

## Qualification matrix

The successor implementation must add canonical fixtures and tests for:

1. no configuration, complete configuration, and incomplete configuration;
2. malformed persisted configuration and stale configuration/catalog
   generations;
3. unavailable provider endpoint, failed authentication, and successful
   authenticated provider authorization;
4. unavailable or unverified agent catalog entries;
5. contradictory facts rejected by both producer and decoder;
6. authenticated owner-only control transport, reconnect, bounded frames, and
   stale/future generation handling; and
7. privacy scans proving serialized output and fixed errors contain no
   credentials, endpoint values, host paths, prompts, or provider payloads.

The focused suite must exercise the real backend source of truth. A mock that
only returns a fabricated `authorized` flag does not qualify the operation.

## Dependency gate

The readiness successor is not implementation-ready until both producer seams
are independently qualified:

1. a verified agent catalog/status producer; and
2. a runtime-owned provider connectivity and authorization result bound to the
   enrolled credential generation and provider catalog generation.

Until then, `ConfigurationSnapshot`, `ProviderCatalog`, and
`AuthStatusResponse` remain separate projections. This boundary is intentional
and is covered by the control-backend regression test
`provider_readiness_remains_unavailable_without_verified_probe`.
