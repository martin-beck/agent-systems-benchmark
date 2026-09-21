# Signed local agent release index

The runner may consume a local `agent-release-index.json` together with the
detached `agent-release-index.json.sig` sidecar beneath its private state root.
The JSON document is signed as-is with SSHSIG namespace
`asb-agent-release-index-v1` and has this closed shape:

```json
{
  "schema_version": 1,
  "runner_instance_id": "runner-identity",
  "target": {"operating_system":"linux","architecture":"x86_64","libc":"glibc","libc_version":"unknown"},
  "agents": []
}
```

Each entry in `agents` is the existing authenticated `AgentCatalogEntry`
shape. The runner validates the signature before parsing, binds the source to
its negotiated runner identity and exact target, recomputes the runtime
catalog digest, and rejects incomplete or contradictory entries. Missing trust
configuration leaves the truthful unavailable roster in place; it never
promotes an unsigned or partially verified entry.

The private runtime configuration supplies the explicit trust root through:

- `ASB_RELEASE_INDEX_SSH_KEYGEN`
- `ASB_RELEASE_INDEX_SSH_KEYGEN_SHA256`
- `ASB_RELEASE_INDEX_ALLOWED_SIGNERS`
- `ASB_RELEASE_INDEX_PRINCIPAL`

These values are runtime configuration only and must not be persisted in the
public state repository, logs, control frames, or benchmark evidence.
