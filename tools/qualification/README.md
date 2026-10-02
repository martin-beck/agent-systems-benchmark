# Development-channel qualification

`current-main-dev-channel.sh` is a disposable AR-1653 consumer check. It
clones the selected ASB and `asb-tui` refs into a private temporary workspace,
builds both release entrypoints with a bounded timeout, enforces a recursive
workspace quota, and publishes a digest-bound `dev` release with a staged
rename. It checks both entrypoints after publication, swaps back to a prior
dev release when one exists, and verifies that an existing `stable` link was
not changed.

The default refs are `main` and the default repositories are the public ASB
and `asb-tui` repositories. Development fixtures and unsigned local builds
are intentional; this check does not require production credentials or
signatures. Override `ASB_DEV_INSTALL_ROOT` to keep the published result for
inspection. The command prints the exact source heads and binary digests in a
transcript path.

Example:

```sh
ASB_DEV_INSTALL_ROOT="$HOME/.local/share/asb-qualification" \
  tools/qualification/current-main-dev-channel.sh
```
