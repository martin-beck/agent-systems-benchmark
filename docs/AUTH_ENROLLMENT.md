# Provider authentication enrollment

`asb-agents::auth` owns the credential-free enrollment record used by provider
connections. A record contains only a stable connection identifier, credential
source, locator digest, generation and public probe status. It never contains a
key, locator, provider response, or filesystem path.

Applications provide a qualified [`SecretBackend`](../crates/asb-agents/src/auth.rs)
implementation for their system secret store or one-shot descriptor/helper.
`enroll_api_key` validates the bounded input and passes it once to that backend;
the enrollment record is safe to persist with `to_json` and restore with
`from_json`. Backends must keep values private and return only
`connected`, `rejected`, `expired` or `unavailable` probe outcomes.

Every probe carries the enrollment generation. A replacement increments the
generation and resets status to `untested`, so a delayed probe cannot restore a
stale credential. Revocation is terminal for the record. No environment,
argument, control-frame or public-evidence fallback is performed.
