# Private development toolchain runner

`setup.sh` stages private, non-group-writable Cargo, Git, and `setsid` entry
points under `ASB_DEV_TOOLCHAIN_ROOT` (default:
`$HOME/.cache/asb/dev-toolchain-v1`). It does not download tools, use
credentials, or alter stable installation behavior. The staged Cargo wrapper
retains the invoking user's pinned Rust toolchain; the manifest records only
bounded tool paths.

Run the ASB development qualification through the staged tools:

```sh
ASB_DEV_TOOLCHAIN_ROOT="$HOME/.cache/asb/dev-toolchain-v1" \
  tools/dev-toolchain/setup.sh
ASB_DEV_TOOLCHAIN_ROOT="$HOME/.cache/asb/dev-toolchain-v1" \
  tools/dev-toolchain/run.sh cargo --version
```

The runner dispatches its staged tools by name. Other commands must be
absolute executable paths; ambient `PATH` command lookup is rejected.

The ASB resolver accepts only absolute paths whose parent chain is private and
whose executable is non-writable by group/other. Missing or unsafe roots report
typed unavailable/invalid diagnostics; arbitrary `PATH` entries are not used.
