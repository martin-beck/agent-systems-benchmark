# ASB agent adapters

The OpenCode adapter runs an exact, hash-pinned OpenCode executable through
`opencode run --format json --pure`. It copies the prompt into a mode-0600 file,
unlinks that file before writing any prompt bytes, rewinds the still-open descriptor,
and passes it directly as standard input. Prompt text is therefore absent from the
filesystem and process argument vector, including when process spawning fails.
The child receives a cleared environment, isolated HOME/XDG
directories, project configuration discovery disabled, sharing disabled, and an
explicit provider endpoint and model. Every attempt receives an atomically new
mode-0700 state directory, so a caller cannot preseed global OpenCode config or
authentication state there. The adapter enables only the in-workspace edit
permission needed by the checked fixture and explicitly denies the wildcard
permission baseline, so shell and every other permission request retain
OpenCode's noninteractive rejection behavior. After the owned process is
reaped, the attempt state tree (including session database, cache, logs, and
prompt file) is removed; cleanup failure makes the attempt fail closed.

The adapter maps OpenCode 1.18.29 JSON events into the versioned ASB event
contract. Completed tool events become an adjacent causal start/finish pair
because this OpenCode CLI format does not emit the tool start in JSON mode.
Text and reasoning content are deliberately not retained. Unknown or malformed
events, inconsistent session IDs, output truncation, version/hash mismatches,
and nonzero exits fail closed without copying raw provider diagnostics.

## Inspected upstream

The implementation was checked against lightweight tag `v1.18.29`, commit
`16747470f976aca3d362ad730bcd3fe82ecc2c9a`, tree
`6d8cc725d9c0945d7259b78e2f60cdec6c493a26`. The upstream MIT `LICENSE` hashes
to `625f0f619133f89bbbb2abe37369613dfa1885eba1e50d02170deb62bb42cb6b`.
The installed `opencode-ai` package metadata hashes to
`946672283f1e84ef0477d3d5a1771c6fad2456c4b86ca03aa144fefdf2ebf89a`;
the exercised executable hash is compiled into the adapter. The lightweight tag
and commit are not cryptographically signed, and this AR does not establish a
reproducible source-to-binary build.

Supported and evidenced:

- Linux x86_64: native OpenCode 1.18.29 completion and cancellation against a
  credential-free loopback OpenAI-compatible fixture.
- Linux aarch64: Rust adapter build/test only; OpenCode package execution is not
  yet natively tested.
- Endpoint override, isolated configuration, structured event mapping, bounded
  prompt/output/event counts, process-group cancellation, and terminal
  tool/usage mapping.

Not yet supported or claimed: Windows/macOS runtime, remote attach mode,
interactive sessions, session resume/fork, live event delivery before process
exit, arbitrary project OpenCode configuration/plugins, automatic permissions,
provider authentication secrecy against same-UID process inspection, or exact
tool-start timing. The verified-executable path and its parent are assumed not
to be replaceable by an attacker between hash verification and `exec`.
Descendants that deliberately leave the owned process group can escape this
adapter's cancellation; hard containment requires the cgroup/sandbox runtime.

### OpenCode replay qualification

The same pinned Linux x86_64 executable is exercised against an in-memory
credential-free capture and the production `StrictReplayService` while the
complete test process runs in a fresh user/network namespace containing only
an enabled loopback interface. The journey prepares the protected
`original.bug-fix` workload, records a transient provider failure, retry, tool
edit, usage and completion, seals the cassette through the standard redactor,
resets the exact workload, and requires replay to reproduce terminal/tool/usage
and independent grader evidence. A separately paced replay is cancelled before
its first response segment and both the agent process group and replay
reservation must terminate cleanly. Malformed, truncated and unknown-field
cassette inputs are rejected before a service starts.

OpenCode generates fresh `x-session-id` and `x-session-affinity` values for each
isolated invocation and changes `x-stainless-retry-count` after a provider
failure. The qualification's declared redaction policy therefore treats only
those three values as volatile correlation data: header presence, every other
header, and the complete semantic JSON request remain strict. No raw capture is
written to disk or committed. This proves offline replay only for the pinned
OpenAI-compatible Chat Completions route, fixture, executable and Linux x86_64
environment. It does not prove live provider recording, other OpenCode/provider
versions, non-loopback transport, native aarch64 execution, or OS timing
determinism. Network denial depends on the surrounding namespace; the replay
service alone is inbound-only and is not a network sandbox.

## aider boundary

The aider adapter targets the universal `aider-chat` 0.86.2 wheel at upstream
commit `253f0368b873ba30d8ee26e463718f0c03614ddf` and verifies both that wheel and
the natively tested CPython 3.12 executable by SHA-256 before import. Upstream
declares Apache-2.0. ASB does not redistribute aider or its Python dependency
environment; the pin establishes compatibility with the inspected top-level
artifact, not a reproducible source build or a complete transitive-dependency
attestation.

Each attempt uses aider's `--message-file` batch mode with commits, Git,
analytics, update checks, release notes, URL detection, browser automation,
linting, tests, prompt caching, streaming, and shell suggestions disabled. The
adapter passes at most 4,096 regular workspace files, rejects links, special
files, files over 16 MiB, and aggregate input over 256 MiB, clears the
environment, isolates HOME/XDG/history/config state,
and deletes prompt-bearing state after the process is reaped. A closed proxy
blocks known auxiliary egress while only the exact provider host bypasses it;
provider scopes that would also bypass a known aider service are rejected.
These proxy controls are defense in depth, not a network sandbox.

Supported and evidenced:

- Linux x86_64, CPython 3.12: native aider 0.86.2 edit completion and
  process-group cancellation against a credential-free loopback
  OpenAI-compatible fixture.
- Explicit provider endpoint/model selection, bounded prompt/output/process
  lifetime, disabled unintended commits, isolated configuration, lifecycle
  events, native exit status, cancellation, and typed unavailable evidence for
  aider-internal retries. A native fixture proves aider retries a transient
  provider failure, but content-bearing human diagnostics are not parsed to
  infer a count.

Not claimed: structured tool, token-usage, or response-content events (aider's
batch output is deliberately discarded); live progress; retry counts or causes;
Windows, macOS, Linux aarch64, musl, or other Python
runtimes; reproducible transitive Python wheels; arbitrary plugins or project
configuration; and same-UID credential secrecy. Hard filesystem, network, and
descendant containment remains the ASB sandbox layer's responsibility.
Workspace and state roots must already resolve to their exact canonical path.
Their ancestors, bounded workspace tree, and verified executable installation
are assumed not to be replaced or expanded between validation and use.

### aider replay qualification

The pinned Linux x86_64 aider 0.86.2 and CPython 3.12 pair is exercised in a
fresh user/network namespace containing only an enabled loopback interface.
The adapter fixes `PYTHONHASHSEED=0` inside its cleared child environment because
this pinned aider release renders editable files from a Python set; the native
qualification compares two independent multi-file captures before strict replay.
An in-memory credential-free OpenAI-compatible fixture returns one buffered
HTTP 500 response followed by a successful whole-file edit. The qualification
normalizes both requests, removes authorization with the standard redaction
policy, seals the cassette, resets the protected `original.bug-fix` workload,
and requires strict replay to reproduce the terminal trajectory and independent
grader result. A separately paced replay is cancelled before its first response
segment, and both the replay reservation and aider process group must terminate
with empty isolated state. Unknown fields, truncated records, and inconsistent
tool declarations are rejected before replay service start.

No raw capture is written to disk or committed. This qualification covers only
the pinned wheel, interpreter, fixture, buffered Chat Completions route, and
Linux x86_64 environment. Aider batch mode does not expose trustworthy
structured tool calls, token usage, or retry counts, so the cassette requires
empty tool declarations and ASB retains typed unavailable retry evidence; these
signals are not inferred from discarded human-readable diagnostics. Replay
controls provider responses, not operating-system timing. Live providers,
non-loopback transport, native aarch64, other aider/Python versions, and timing
determinism remain unsupported. Network denial is supplied by the surrounding
namespace rather than the inbound-only replay service.

## Goose boundary

The Goose adapter targets AAIF Goose 1.49.0 through
`goose run --no-session --no-profile --with-builtin developer --quiet
--output-format stream-json`. It passes instructions through an unlinked
mode-0600 descriptor on standard input, clears the inherited environment,
uses an isolated per-attempt `GOOSE_PATH_ROOT`, disables keyring access and
session naming, sets the context-file list to empty, and supplies the provider,
model, endpoint, maximum turns, and repeated-tool ceiling explicitly. Only the
bundled `developer` extension is requested. The configured executable,
workspace, and state root must already be exact canonical non-symlink paths.
For each attempt, the adapter opens the configured executable without following
the final symlink, copies and hashes those same opened bytes into a new
mode-0500 file inside the private attempt directory, and launches only that
verified private artifact. Replacing the configured path after the copy cannot
change the launched bytes.

Goose 1.49.0 continues with exit zero when a requested extension fails to
start. A successful quiet run is therefore required to have empty diagnostic
stderr; any warning, including an extension-start warning, fails closed. The
same version also converts provider failures such as an HTTP 400 into an
assistant diagnostic followed by a zero-exit completion. Provider-generated
assistant messages carry inference metadata; a non-inference assistant
diagnostic is therefore mapped to a privacy-filtered failed terminal outcome,
even when the native exit code is zero. The diagnostic text is not retained.
The adapter accepts only bounded unique-key JSONL with causal tool
request/response identities and a single terminal completion. Caller
correlation IDs, each upstream line and message-content array, total upstream
events, and independently retained ASB events all have hard byte or cardinality
ceilings. It retains lifecycle, tool name,
tool success, and input/output token counts, but discards response text,
reasoning, tool arguments/results, upstream errors, session identifiers, and
floating cost. Events are collected after process exit, so live streaming is
not claimed.

The inspected lightweight tag `v1.49.0` resolves to commit
`71fc4be1ed729e26b1dc0a4466abdd03be548a53` and tree
`448f24c739dae4998c583b2b7dc13c6601c155ba`. Neither tag nor commit is
cryptographically signed. Upstream declares Apache-2.0; the inspected
`LICENSE` SHA-256 is
`44459b86c2e96fdbfd8a6b5c33d30d4b04b5293fcb2ec96fe4dcc4e0f90b8962`.
The official x86_64 and aarch64 musl archive and extracted executable digests
are recorded in the committed provenance fixture. GitHub's verifier accepted
the release's SLSA v1 attestation for both archives while enforcing repository,
tag ref, source commit, and GitHub-hosted runner identity; the signer was
`.github/workflows/release.yml` at the same tag and source commit in run
`33792982006`. The attestation binds the published archives to that workflow,
but ASB has not independently reproduced the source-to-binary build or audited
the complete transitive dependency graph. ASB does not redistribute Goose.

Supported and evidenced:

- Linux x86_64 with the pinned static musl release: native tool edit,
  structured lifecycle/usage mapping, provider override, no-session isolation,
  process-group cancellation, cleanup, and extension-failure rejection against
  a credential-free loopback OpenAI-compatible fixture.
- Linux aarch64 with the corresponding pinned static musl release only after
  its exact native CI journey passes; the archive and executable pins alone are
  provenance, not native evidence.
- Explicit HTTP loopback or HTTPS OpenAI-compatible endpoints, bounded prompt,
  output, event, turn, and repeated-tool counts. Proxy settings are defense in
  depth; endpoints whose `NO_PROXY` suffix scope would also exempt the
  inspected Goose 1.49.0 `us.i.posthog.com` telemetry destination are rejected.
  Hard network and filesystem containment remains the ASB sandbox.

Not claimed: interactive or persisted sessions, profiles, recipes, arbitrary
MCP/extensions, ambient hints or skills, ACP/subscription providers, remote
attach, live events, monetary-cost evidence, GPU/Vulkan, Windows/macOS runtime,
glibc-specific release assets, or Alpine/native-musl host validation.
Descendants that leave the owned process group require cgroup containment.
The canonical workspace and state roots are assumed not to be replaced during
an attempt.

The credential-free native replay qualification records the pinned Goose
binary's model-catalog probes and OpenAI-compatible chat-completion traffic in
memory, redacts authorization plus complete message arrays before sealing, and
then replays the same cassette through the strict loopback service while the
real Goose binary and workload tools execute again. It proves one bounded
synthetic HTTP 429 retry, workspace-relative file editing, tool/result
causality, independent grading parity, paced cancellation, malformed and
truncated cassette rejection, and loopback-only network reachability. This
controls provider responses; it does not claim deterministic process timing or
native support beyond the pinned Linux x86_64 musl artifact.

## Provider-profile binding

The shared binding interface negotiates a complete version/provider/endpoint/
credential/setting/transport capability matrix before calling adapter translation.
Translation must remain pre-start and expose the exact effective credential-free
profile; a constructor-controlled proof is returned only when its canonical digest
matches the requested profile. Current concrete adapters intentionally do not
implement this interface until their provider-specific ARs prove real configuration
output, so the generic boundary makes no live-provider compatibility claim.

The credential boundary resolves only an explicitly configured `environment`,
`file_descriptor`, or `helper` reference. Each SHA-256 binds a versioned source
kind and public logical locator, never credential bytes, descriptor numbers, or
paths. Environment resolution reads one allowlisted uppercase variable. The FD
resolver consumes an already-open `OwnedFd` once after checking current-user
ownership, regular-file type, private mode, and the byte bound both at admission
and immediately before reading. The caller remains responsible for safe opening;
no pre-open path-traversal claim is made.

The helper resolver likewise accepts an already-open current-user executable,
requires no group/world write permission plus an exact content SHA-256, and copies the
verified bytes into a CLOEXEC, write/grow/shrink sealed in-memory image before
returning from admission. Later source mutation or replacement cannot change the
launched image, and the staged descriptor is never made inheritable in the parent.
The identity is bound into the reference digest. It invokes only the fixed v1 argument with
an empty environment and null input, under bounded stdout, stderr, deadline,
termination, and process-group cleanup. The exact JSON response contains only
`version` and `credential`; unknown fields, malformed or oversized output,
stderr, nonzero exit, timeout, and cancellation all fail without returning raw
output. Resolved values remain non-cloneable and non-serializable and are
consumed when spawning one bounded provider process. No resolver falls back to
ambient credentials or places credential bytes in argv or an ASB evidence type.
