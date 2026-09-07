# mini-SWE-agent adapter boundary

ASB targets `mini-swe-agent==2.4.6`, official lightweight tag and commit
`a83fcae82d2a08f0ee0c688f9d137b3566c097f8`, tree
`665df42f5761252b83d9a30e1b82f76f1f17f828`. The tag and commit are not
cryptographically signed. The inspected MIT license is present in source and wheel metadata.
The independently downloaded PyPI wheel SHA-256 is
`a35463c553ac825c7773b03cfa69cd44958e3af20155dcc5711fdf9e4c67cd54`;
the sdist SHA-256 is
`0532c8193a763409fa52bb2b5a5d7ac9052dcb1c2cae43945b14b1b7f6ba869a`.

The adapter verifies the wheel and the tested CPython 3.12 executable before
starting. It extracts the verified wheel into private attempt state, imports
that copy with dependencies from the caller-provided Python environment, and
validates the saved `mini-swe-agent-1.1` trajectory. Upstream ships dependency
ranges, not a complete lock; its LiteLLM constraint excludes compromised
versions 1.82.7 and 1.82.8, but ASB does not claim reproducible or fully
attested transitive Python dependencies.

Prompts enter through an unlinked mode-0600 descriptor and are absent from the
argument vector and durable prompt files. HOME, XDG roots, mini-SWE global
configuration, and temporary storage are fresh per attempt; the inherited
environment is cleared. Auxiliary HTTP(S) proxy routes fail closed and only
the exact explicit provider host bypasses them. This is defense in depth, not
a network sandbox. Workspace traversal accepts at most 4,096 regular files,
16,384 entries, 16 MiB per file, 256 MiB total, and rejects symlinks and
special files. The driver caps model actions at 4,096, individual shell calls
at 30 seconds, and total process time at the caller's bounded deadline.

Mapped evidence includes causal lifecycle, bash tool start/finish and bounded
USD cost from the private trajectory. Prompt, reasoning, commands, tool output,
submission text, and raw diagnostics are discarded. Duplicate JSON members,
unknown roles, duplicate/unmatched tool IDs, excessive counts or bytes,
invalid usage, version mismatch, truncation, and malformed terminal state fail
closed. A successful submission command has no upstream tool observation; the
adapter closes that single final causal pair only when the pinned trajectory
terminal status is `Submitted`.

Native evidence is limited to Linux x86_64 with the pinned CPython runtime,
verified 2.4.6 wheel, and credential-free loopback OpenAI-compatible fixture.
Linux aarch64 is build/test only until a native package journey is recorded.
Windows, macOS, musl, interactive mode, arbitrary project configuration,
plugins, remote subscription/attach, session resume, and ASB replay routes are
unsupported. Live streaming is not claimed: trajectory events become visible
only after process exit. Same-UID process inspection, hard filesystem/network
containment, and descendants that deliberately escape the owned process group
require the ASB sandbox/cgroup layer.
