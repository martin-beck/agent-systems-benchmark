# Frontend control API v1

The ASB runner and a frontend have different lifetimes. The runner owns durable
plans, run processes, journals, artifacts, and recovery. A frontend owns only a
connection, its outstanding request IDs, and reconnect cursors. Closing the
frontend therefore has no run-lifecycle effect.

## Local transport

Local v1 uses a Linux Unix-domain stream socket. Each body is UTF-8 JSON-RPC
2.0 preceded by a four-byte unsigned big-endian length. Empty, oversized,
truncated, malformed, trickle-fed, or deadline-expired frames are rejected
before backend work. The runtime directory must be a real same-user directory
with no group or other permissions. ASB refuses every existing socket path, binds mode 0600, and
checks accepted peers with `SO_PEERCRED`. Cleanup removes the node only if its
device and inode still identify the socket ASB created.
The frontend also checks the connected server's kernel credentials before it
sends negotiation data. Kernel-derived peer identities expose read-only
accessors and cannot be forged through the public API.

There is no automatic IP listener, including when the process runs under SSH,
tmux, or screen. Remote control is separately configured, authenticated, and
audited.

## Negotiation and flow control

The first call is `negotiate`. Its complete JSON-RPC envelope, ID, timeout,
version set, and limits are validated before session state changes. Client and
runner select an exact version present in both supported sets, then take the
stricter frame, deadline, page, and in-flight limits. A client offering only a
newer minor is rejected rather than silently assigned a version it did not
offer. Calls before negotiation, renegotiation, duplicate outstanding request
IDs, and capacity exhaustion fail before external effects.
Every admitted call captures one absolute monotonic processing deadline. The
same diminishing budget bounds all socket reads/writes and is passed to the
backend; partial traffic never resets it. The trusted runner backend must check
that deadline before every mutation commit or durably record that it needs
reconciliation. The endpoint never detaches backend work or emits a late
success. A backend that violates the cooperative contract can occupy only one
bounded connection worker until it returns; the service continues admitting
other connections within its negotiated worker ceiling.

The v1 methods are:

| Method | Purpose | Mutation |
| --- | --- | --- |
| `negotiate` | Select version and limits | no |
| `capabilities` | Describe runner and transport support | no |
| `validate_settings` | Validate without creating a run | no |
| `create_plan` | Persist a validated content-pinned plan | yes |
| `launch` | Launch one durable plan | yes |
| `status` | Read authoritative run and attempt state | no |
| `cancel` | Cancel one exact run/attempt | yes |
| `history` | Page through recent durable runs | no |
| `repeat` | Create a new plan from an immutable prior run | yes |
| `analyze` | Analyze a bounded run set | no |
| `events` | Resume public events after a durable revision | no |
| `artifact_metadata` | Read digest, size, and sensitivity only | no |

## Durable mutation and recovery

Each mutation carries an opaque bounded idempotency key. The runner canonicalizes
the request and durably journals its digest and terminal public result. Only
after that commit may the key enter the retry index. The same key and digest
replays the committed result; a different digest conflicts; exhausted retention
fails closed instead of evicting knowledge needed to prevent duplication.
After restart, the index is rebuilt from the authoritative journal. An uncertain
external effect remains `needs_reconciliation` and is never silently repeated.
Catalog recovery cross-checks every committed plan reference against its canonical
plan ID and digest, every launch result against the exact run, attempt, plan digest,
and initial event revision, and every committed cancellation against a cancelled
authoritative run. Contradictory durable records fail closed.

`cancel` includes both run and exact attempt IDs. A stale attempt cannot cancel
its successor. Success acknowledges durable cancellation state, not merely that
a signal was sent.

## Events, pagination, and privacy

Public events have contiguous durable revisions. A page contains a bounded
ordered prefix, a next cursor, and `has_more`. A cursor older than retained
history is stale; one newer than runner truth is invalid. Neither condition is
reported as an empty successful page. Independent clients advance independent
cursors, and reconnect cannot create a mutation.

Run history is ordered by each run's immutable `created_revision`; its page
cursor uses that value even when a returned summary's current `revision`
advances between requests. State changes therefore cannot duplicate or displace
a run across history pages. A summary always requires
`created_revision <= revision`.

Ordinary successful responses use a closed typed result vocabulary bound to the
request method. It contains only stable IDs, states, revisions, plan or artifact
digests, sizes, bounded issue enums, booleans, and explicit sensitivity.
Identity, digest, page ordering, cursor, and success invariants are checked
before framing and again by the client. They contain no host paths, prompt or
response bodies, credentials, or private logs.
Sensitive artifact content requires a separate least-privilege authorization
boundary.

The canonical Rust types, runner endpoint, reusable frontend client, generated
JSON Schemas, public fixtures, bounded state models, transport/lifecycle tests,
and retained negative cases live in `crates/asb-control` and
`formal/tests/control_models.rs`. End-to-end tests prove disconnect does not
cancel a launched run, endpoint restart plus retry does not duplicate it,
malformed negotiation cannot reach the backend, invalid typed backend output is
not written, trickled ingress consumes the same absolute request budget, and
late backend outcomes are joined and rejected without post-response effects.
