# Delegated strict-replay runner payload

The image contains Bubblewrap 0.9.0 and systemd 255.4, but an image cannot
create a systemd manager or grant kernel namespace capability. The launcher
therefore fails closed when those capabilities are absent. It never uses
`--privileged`, host `/sys` or `/run`, host networking, credentials, or broad
mounts. Runtime bounds are network-none, read-only root, all capabilities
dropped, no-new-privileges, 64 processes, 256 MiB, one CPU, tmpfs `/run` and
`/tmp`, and one read-only workspace bind.

Build from the pinned multi-architecture Ubuntu index after package inputs are
reviewed and cached:

```sh
docker build --pull=false --file tools/qualification-runner/image/Dockerfile \
  --build-arg SOURCE_REVISION=<reviewed-revision> \
  --tag asb-replay-runner-v1:0.1.0 .
```

Qualification requires a disposable VM or explicitly approved host runner with
a running systemd manager and user namespaces. Publish only a signed,
multi-architecture manifest after each platform digest and package provenance
are reviewed; the local Docker build is not publication or approval evidence.
