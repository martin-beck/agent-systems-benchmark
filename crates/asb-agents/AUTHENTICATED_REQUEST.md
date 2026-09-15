# Authenticated provider request v1

`AuthenticatedRequest` is credential-free metadata. Its endpoint identity is the SHA-256 digest
of the exact endpoint string, and generation, timeout, and response bounds are mandatory. The
opaque `ResolvedCredential` is consumed only by `inject`, which validates endpoint and generation,
checks cancellation before and after the final `HeaderSink`, and wipes its temporary buffer.

The versioned JSON shape is `schema/authenticated-request-v1.schema.json`; unknown fields are
rejected. AR-1228 consumes this seam for bounded provider transport. AR-1229 owns durable config,
control, and CLI integration. No credential value may enter this schema, state, logs, or evidence.
