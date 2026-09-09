#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu
ROOT=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
SCRIPT=$ROOT/tools/install/bootstrap.sh
LIFECYCLE=$ROOT/tools/install/lifecycle.sh
test -f "$SCRIPT"
test "$(head -n 3 "$SCRIPT" | tail -n 1)" = '# SPDX-License-Identifier: MIT'
sh -n "$SCRIPT"
sh -n "$LIFECYCLE"
if ASB_MANIFEST_URL=http://example.invalid ASB_MANIFEST_SHA256= \
    ASB_DATA_ROOT=/tmp/asb-test ASB_CONFIG_ROOT=/tmp/asb-test-config \
    ASB_RUNTIME_ROOT=/tmp/asb-test-run "$SCRIPT" >/dev/null 2>&1; then
    echo 'insecure manifest URL unexpectedly accepted' >&2
    exit 1
fi
if ASB_MANIFEST_URL=https://example.invalid/manifest \
    ASB_MANIFEST_SHA256=not-a-digest ASB_DATA_ROOT=/tmp/asb-test \
    ASB_CONFIG_ROOT=/tmp/asb-test-config ASB_RUNTIME_ROOT=/tmp/asb-test-run \
    "$SCRIPT" >/dev/null 2>&1; then
    echo 'invalid manifest digest unexpectedly accepted' >&2
    exit 1
fi
printf 'bootstrap negative-path tests passed\n'

BASE=/srv/data/projects/.asb-local/asb-install-test-$$
trap 'rm -rf -- "$BASE"' EXIT HUP INT TERM
mkdir -p "$BASE/payload" "$BASE/fake-bin"
printf '%s\n' '#!/bin/sh' 'test "${1:-}" = doctor' > "$BASE/payload/asb"
chmod 0755 "$BASE/payload/asb"
(cd "$BASE/payload" && tar -czf "$BASE/asb.tar.gz" asb)
ARCHIVE_SHA=$(sha256sum "$BASE/asb.tar.gz" | awk '{print $1}')
ARCHIVE_SIZE=$(stat -c '%s' "$BASE/asb.tar.gz")
printf '%s\n' fixture-signature > "$BASE/asb.tar.gz.sig"
SIGNATURE_SHA=$(sha256sum "$BASE/asb.tar.gz.sig" | awk '{print $1}')
python3 - "$BASE" "$ARCHIVE_SHA" "$ARCHIVE_SIZE" "$SIGNATURE_SHA" <<'PY'
import json
import platform
import sys
from hashlib import sha256

base, digest, size, signature = sys.argv[1:]
libc, version = platform.libc_ver()
architecture = {"amd64": "x86_64", "aarch64": "aarch64"}.get(platform.machine(), platform.machine())
manifest = {
    "schema_version": 1,
    "source_revision": "a" * 40,
    "expires_at": "2999-01-01T00:00:00Z",
    "protocol_min": 1,
    "protocol_max": 1,
    "artifacts": [{
        "artifact_id": "fixture-v1",
        "target": {"operating_system": platform.system().lower(), "architecture": architecture,
                    "libc": libc or "unknown", "libc_version": version},
        "url": "https://downloads.example.invalid/fixture.tar.gz",
        "sha256": digest,
        "size": int(size),
        "signature_sha256": signature,
    }],
}
raw = json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode() + b"\n"
open(base + "/manifest.json", "wb").write(raw)
open(base + "/manifest.sha256", "w").write(sha256(raw).hexdigest())
PY
cat > "$BASE/fake-bin/curl" <<'EOF'
#!/bin/sh
set -eu
out=
url=
while test "$#" -gt 0; do
    case "$1" in
        --output) out=$2; shift 2 ;;
        https://*) url=$1; shift ;;
        *) shift ;;
    esac
done
test -n "$out"
case "$url" in
    *manifest.json) cp "$ASB_INSTALL_FIXTURE/manifest.json" "$out" ;;
    *fixture.tar.gz) cp "$ASB_INSTALL_FIXTURE/asb.tar.gz" "$out" ;;
    *fixture.tar.gz.sig) cp "$ASB_INSTALL_FIXTURE/asb.tar.gz.sig" "$out" ;;
    *) exit 1 ;;
esac
EOF
chmod 0755 "$BASE/fake-bin/curl"
printf '%s\n' '#!/bin/sh' 'exit 0' > "$BASE/fake-bin/ssh-keygen"
chmod 0755 "$BASE/fake-bin/ssh-keygen"
printf '%s\n' fixture-signer > "$BASE/allowed-signers"
ASB_INSTALL_FIXTURE=$BASE PATH="$BASE/fake-bin:$PATH" \
    ASB_MANIFEST_URL=https://downloads.example.invalid/manifest.json \
    ASB_MANIFEST_SHA256=$(cat "$BASE/manifest.sha256") ASB_DATA_ROOT="$BASE/data" \
    ASB_CONFIG_ROOT="$BASE/config" ASB_RUNTIME_ROOT="$BASE/run" ASB_SERVICE_MODE=disabled \
    ASB_ALLOWED_SIGNERS_FILE="$BASE/allowed-signers" ASB_SIGNATURE_PRINCIPAL=fixture \
    ASB_NO_TUI=1 "$SCRIPT" > "$BASE/install.out"
test -L "$BASE/data/current"
test -x "$BASE/data/current/asb"
test -f "$BASE/config/config.toml"
test "$(stat -c '%a' "$BASE/config/config.toml")" = 600
ASB_DATA_ROOT="$BASE/data" ASB_CONFIG_ROOT="$BASE/config" ASB_RUNTIME_ROOT="$BASE/run" "$LIFECYCLE" status | grep -F 'current=releases/fixture-v1'
ASB_DATA_ROOT="$BASE/data" ASB_CONFIG_ROOT="$BASE/config" ASB_RUNTIME_ROOT="$BASE/run" "$LIFECYCLE" backup >/dev/null
test -d "$BASE/data/backups"
ASB_DATA_ROOT="$BASE/data" ASB_CONFIG_ROOT="$BASE/config" ASB_RUNTIME_ROOT="$BASE/run" "$LIFECYCLE" repair
ASB_DATA_ROOT="$BASE/data" ASB_CONFIG_ROOT="$BASE/config" ASB_RUNTIME_ROOT="$BASE/run" "$LIFECYCLE" uninstall | grep -F "retained-data=$BASE/data"
test ! -e "$BASE/data/current"
printf 'bootstrap positive installation test passed\n'
