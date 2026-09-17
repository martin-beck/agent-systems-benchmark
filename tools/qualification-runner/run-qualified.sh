#!/bin/sh
set -eu
# Never add --privileged implicitly: nested namespace support requires an
# explicitly approved VM or host runner and Docker must fail closed here.
ROOT=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
IMAGE=${ASB_REPLAY_IMAGE:-asb-replay-runner-v1:0.1.0}
command -v docker >/dev/null 2>&1 || { echo "runner-unavailable: docker is absent" >&2; exit 78; }
docker image inspect "$IMAGE" >/dev/null 2>&1 || { echo "runner-unavailable: image is not locally installed: $IMAGE" >&2; exit 78; }
exec docker run --rm --network none --read-only --cap-drop ALL \
  --security-opt no-new-privileges --pids-limit "${ASB_PIDS_LIMIT:-64}" \
  --memory "${ASB_MEMORY_LIMIT:-256m}" --cpus "${ASB_CPUS:-1}" \
  --tmpfs /run:rw,nosuid,nodev,noexec,size=16m \
  --tmpfs /tmp:rw,nosuid,nodev,noexec,size=16m \
  -v "$ROOT/workspace:/workspace:ro" "$IMAGE" capability-probe
