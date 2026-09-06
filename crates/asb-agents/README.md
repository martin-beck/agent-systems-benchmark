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
