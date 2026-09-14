# Optional TUI lifecycle

`asb` does not contain the TUI application, renderer, terminal state, or
Ratatui/Crossterm integration. Those remain in the independent
[`martin-beck/asb-tui`](https://github.com/martin-beck/asb-tui) program. ASB is
only the trusted, rootless lifecycle router:

```sh
asb tui install
asb tui status
asb tui                 # same as: asb tui launch
asb tui upgrade
asb tui doctor
asb tui remove
```

The install and upgrade commands accept `--offline`, `--dry-run`, and
`--launch`. `--dry-run --launch` is contradictory and is rejected. A dry run
authenticates and validates every input but neither executes nor activates the
candidate. `--version` reports the router version.

The router remains intentionally renderer-neutral: terminal drawing, keymaps,
screens, and report presentation are delegated to the separately installed
`asb-tui` executable through the authenticated handoff contract.

An ASB checkout remains usable without terminal UI dependencies; installing
`asb-tui` is an explicit opt-in operation, and UI implementation changes belong
in the standalone repository.

The lifecycle contract is versioned independently from the TUI presentation
layer, allowing each repository to release and validate on its own cadence.

The authenticated handoff remains the sole integration seam between the
benchmark runner and the separately shipped UI.

Both sides can validate this seam independently before a release is promoted.

GitHub merge metadata is retained separately from the signed topic history for auditability.

DCO identity matching follows the canonical Git author string used by hosted policy.

Each promotion records the exact source commit and tree before artifact publication.

## Trust and transaction boundary

Online acquisition uses fixed HTTPS channel URLs, a fixed embedded SSH signer,
an independent channel-signature namespace, bounded manual redirects, and only
immutable versioned release URLs. ASB verifies the signed channel, release and
target compatibility, signed manifest, all five artifact digests, license
report, SPDX SBOM, provenance, and exact component identities before any
candidate byte executes. It then executes those exact bytes from a sealed
anonymous descriptor. The candidate independently reverifies and atomically
activates its installation.

The signed channel record, manifest, and signatures are retained under the
durable state root. Every later network-free status, doctor, remove, or launch
reverifies both namespaces, proves that the manifest was promoted by that
channel, and binds the active release, source, compatibility, and executable
digest before executing the installed bytes. Channel and manifest validity time
is an acquisition/promotion gate: expired records cannot install or upgrade,
while an installation accepted during their validity remains locally usable
after expiry because reauthentication is structural and cryptographic, not a
hidden network freshness check.

ASB writes a crash-durable monotonic pending floor after all signed inputs are
verified and before candidate activation, then promotes it to accepted state
only after an exact operation-bound success response. Clearing download cache
or removing the TUI cannot clear downgrade/substitution protection. A stable
channel without a currently promoted `verified_extension` release fails closed;
source-only builds are not installable releases.

The copied v1 lifecycle and bundle schemas under
`crates/asb-cli/schema/tui/v1/` are byte-pinned to the hardened asb-tui contract
at `d58eda9b74431752675f784978dd5851f4eb9f18`. Interactive launch qualification
is pinned separately in `upstream-contract.json`: the frontend opens and
validates `/dev/tty` for its own standard streams while lifecycle JSON stays on
the parent pipes. This prevents terminal output from corrupting the delegated
response. The qualification pin remains empty and non-qualifying until a
repaired exact head passes hosted CI and independent review. ASB never owns,
signals, or replaces benchmark runner processes.

The lifecycle candidate runs in a router-owned process group. Once its leader
exits, ASB closes any lifecycle-response pipe retained by a frontend descendant
before reaping the leader. All `asb-tui` descendants are frontend-owned and
must terminate with that frontend operation. A frontend child intended to
outlive it is a contract violation. Benchmark runners are never frontend
descendants: `asb-tui` only talks to ASB's separately owned authoritative
runner, whose PID and process group are outside this cleanup boundary.

## Rootless storage and network behavior

The three roots intentionally have different lifetimes:

- installed versions: `$XDG_DATA_HOME/asb/extensions/asb-tui`
- durable acceptance ledger and lock: `$XDG_STATE_HOME/asb/extensions/asb-tui`
- resumable/download cache: `$XDG_CACHE_HOME/asb/asb-tui`

Missing variables use the standard `~/.local/share`, `~/.local/state`, and
`~/.cache` fallbacks. Directories are owner-private. File operations retain
directory descriptors for each atomic operation, reject symlink traversal, use
unique exclusive temporary files, sync file contents, rename atomically, and
sync the parent.

These rootless controls protect against accidental corruption, cache clearing,
path substitution outside the retained operation, and interrupted concurrent
lifecycle commands. They do not claim a monotonic hardware trust anchor against
a hostile process already running as the same user, which can rewrite all XDG
state and replay an older legitimately signed release. Cryptographic
reauthentication still prevents such state edits from selecting arbitrary
unsigned executable bytes.

`status`, `doctor`, `remove`, and `launch` never use the network. Their JSON
field `network` is therefore `denied`. `doctor` on an absent installation is a
successful diagnostic (`exit 0`, `extension_not_installed`). Absent `status`,
`remove` (including repeated remove), and `launch` are stable negative results
(`exit 3`, `extension_not_installed`). Operational failures use exit 4; usage
errors use the ordinary ASB usage exit.

Offline install/upgrade consumes only previously authenticated cache inputs,
but the durable rollback floor always remains under `XDG_STATE_HOME`. Online
install/upgrade and their dry runs report `network: used`; an install/upgrade
followed by `--launch` retains that network evidence in its combined result.
