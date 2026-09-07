#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
if test "$(id -u)" -ne 0; then
    exec sudo -n --preserve-env=PATH "$0" "$@"
fi
if ! awk '$2 == "/proc" && $4 ~ /(^|,)(hidepid=2|hidepid=invisible)(,|$)/ { found=1 } END { exit !found }' /proc/mounts; then
    test "${ASB_PRIVATE_PROC_FIXTURE:-}" != 1 || { echo 'private procfs fixture setup failed' >&2; exit 1; }
    exec unshare --mount --propagation private sh -c \
        'mount -t proc proc /proc -o remount,hidepid=2 && ASB_PRIVATE_PROC_FIXTURE=1 exec "$@"' sh "$0" "$@"
fi
ROOT=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
cd "$ROOT"
test_parent=/srv/data/projects/.asb-local
mkdir -p "$test_parent"
ASB_STORAGE_ROOT=$(mktemp -d "$test_parent/asb-runner-storage-fixture-XXXXXXXXXXXX")
export ASB_STORAGE_ROOT
test_storage=$ASB_STORAGE_ROOT
cleanup() {
    case "$test_storage" in
        /srv/data/projects/.asb-local/asb-runner-storage-fixture-*) rm -rf -- "$test_storage" ;;
        *) echo 'refusing unsafe test cleanup' >&2; exit 1 ;;
    esac
}
trap cleanup EXIT HUP INT TERM
service_uid=$(id -u nobody)
service_gid=$(id -g nobody)
export ASB_OPERATOR_UID=0 ASB_OPERATOR_GID=0 ASB_SERVICE_UID=$service_uid ASB_SERVICE_GID=$service_gid
chown 0:"$service_gid" "$ASB_STORAGE_ROOT"
chmod 0750 "$ASB_STORAGE_ROOT"
mkdir "$ASB_STORAGE_ROOT/asb-ci-runners"
chown 0:"$service_gid" "$ASB_STORAGE_ROOT/asb-ci-runners"
chmod 0750 "$ASB_STORAGE_ROOT/asb-ci-runners"
ASB_RUNNER_ROOT=$ASB_STORAGE_ROOT/asb-ci-runners/asb-test-0123456789ab
export ASB_RUNNER_ROOT
export ASB_RUNNER_NAME=asb-runner-0123456789ab
export ASB_RUNNER_CONFIG_LABELS=asb-development-v1-x86_64-ubuntu2404
mkdir "$ASB_RUNNER_ROOT"
chown 0:"$service_gid" "$ASB_RUNNER_ROOT"
chmod 0750 "$ASB_RUNNER_ROOT"
mkdir -p "$ASB_RUNNER_ROOT/control" "$ASB_RUNNER_ROOT/runner/bin"
chmod 0700 "$ASB_RUNNER_ROOT/control"
printf 'runner\n' > "$ASB_RUNNER_ROOT/runner/bin/Runner.Listener"
cat > "$ASB_RUNNER_ROOT/runner/config.sh" <<'EOF'
#!/bin/sh
set -eu
for state in .runner .credentials .credentials_rsaparams; do
    printf 'ephemeral-fixture\n' > "$(dirname "$0")/$state"
done
if test -e "$(dirname "$0")/_work/fail-register"; then
    rm -f "$(dirname "$0")/.credentials" "$(dirname "$0")/.credentials_rsaparams"
    exit 1
fi
EOF
printf '#!/bin/sh\nexit 0\n' > "$ASB_RUNNER_ROOT/runner/run.sh"
printf 'ACTIONS_RUNNER_REQUIRE_JOB_CONTAINER=true\n' > "$ASB_RUNNER_ROOT/runner/.env"
chown -R 0:"$service_gid" "$ASB_RUNNER_ROOT/runner"
find "$ASB_RUNNER_ROOT/runner" -type d -exec chmod 0750 {} +
chmod 0550 "$ASB_RUNNER_ROOT/runner/bin/Runner.Listener" "$ASB_RUNNER_ROOT/runner/config.sh" "$ASB_RUNNER_ROOT/runner/run.sh"
chmod 0440 "$ASB_RUNNER_ROOT/runner/.env"
for mutable in runner/_work runner/_diag cache artifacts tmp; do
    install -d -m 0700 -o "$service_uid" -g "$service_gid" "$ASB_RUNNER_ROOT/$mutable"
done
ASB_RUNNER_TOOLS_DIR=$ROOT/tools/runners
export ASB_RUNNER_TOOLS_DIR
. "$ASB_RUNNER_TOOLS_DIR/common.sh"
test "$PATH" = /usr/sbin:/usr/bin:/sbin:/bin
{
    printf '%s %s %s %s\n' "$ASB_RUNNER_VERSION" "$ASB_RUNNER_NAME" "$ASB_RUNNER_LABELS" "$ASB_RUNNER_LINUX_X64_SHA256"
    installation_inventory
} > "$ASB_RUNNER_ROOT/control/manifest"
printf 'tester %s\n' "$(( $(date +%s) + 600 ))" > "$ASB_RUNNER_ROOT/control/lease"
chmod 0600 "$ASB_RUNNER_ROOT/control/lease" "$ASB_RUNNER_ROOT/control/manifest"
export ASB_REPOSITORY=martin-beck/agent-systems-benchmark
ln -s /etc/passwd "$ASB_RUNNER_ROOT/runner/installation-escape"
if installation_inventory > /dev/null 2> "$ASB_RUNNER_ROOT/control/inventory-error"; then
    echo 'escaping installation symlink unexpectedly passed' >&2
    exit 1
fi
grep -Fx 'ERROR: installation inventory failed' "$ASB_RUNNER_ROOT/control/inventory-error"
if grep -q /srv/ "$ASB_RUNNER_ROOT/control/inventory-error"; then
    echo 'installation error leaked a path' >&2
    exit 1
fi
rm -f "$ASB_RUNNER_ROOT/runner/installation-escape"
truncate -s 268435457 "$ASB_RUNNER_ROOT/runner/oversized-installation"
chown 0:"$service_gid" "$ASB_RUNNER_ROOT/runner/oversized-installation"
chmod 0440 "$ASB_RUNNER_ROOT/runner/oversized-installation"
if installation_inventory > /dev/null 2> "$ASB_RUNNER_ROOT/control/inventory-error"; then
    echo 'oversized installation file unexpectedly passed' >&2
    exit 1
fi
grep -Fx 'ERROR: installation inventory failed' "$ASB_RUNNER_ROOT/control/inventory-error"
rm -f "$ASB_RUNNER_ROOT/runner/oversized-installation" "$ASB_RUNNER_ROOT/control/inventory-error"
printf 'proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n' > "$ASB_RUNNER_ROOT/control/proc-insecure"
if procfs_has_private_pids "$ASB_RUNNER_ROOT/control/proc-insecure"; then
    echo 'visible procfs fixture unexpectedly passed' >&2
    exit 1
fi
printf 'proc /proc proc rw,nosuid,nodev,noexec,relatime,hidepid=2 0 0\n' > "$ASB_RUNNER_ROOT/control/proc-private"
procfs_has_private_pids "$ASB_RUNNER_ROOT/control/proc-private"
touch "$ASB_RUNNER_ROOT/runner/.credentials"
if printf 'valid-token\n' | tools/runners/register.sh >/dev/null 2>&1; then
    echo 'partial registration state unexpectedly passed' >&2
    exit 1
fi
rm -f "$ASB_RUNNER_ROOT/runner/.credentials"
touch "$ASB_RUNNER_ROOT/runner/_work/fail-register"
chown "$service_uid:$service_gid" "$ASB_RUNNER_ROOT/runner/_work/fail-register"
if printf 'failure-token\n' | tools/runners/register.sh >/dev/null 2>&1; then
    echo 'failed registration unexpectedly passed' >&2
    exit 1
fi
for state in .runner .credentials .credentials_rsaparams; do
    test ! -e "$ASB_RUNNER_ROOT/runner/$state"
done
rm -f "$ASB_RUNNER_ROOT/runner/_work/fail-register"
oversized=$(awk 'BEGIN {for (i=0; i<1025; i++) printf "a"}')
if printf '%s\n' "$oversized" | tools/runners/register.sh >/dev/null 2>&1; then
    echo 'oversized registration token unexpectedly passed' >&2
    exit 1
fi
if printf 'valid-token\ntrailing\n' | tools/runners/register.sh >/dev/null 2>&1; then
    echo 'registration trailing data unexpectedly passed' >&2
    exit 1
fi
printf 'valid-token\n' | tools/runners/register.sh | grep -q 'registered ephemeral runner'

tools/runners/health.sh | grep -q '^healthy version=2.337.0 '
chmod 0755 "$ASB_RUNNER_ROOT/runner"
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'runner directory mode drift unexpectedly passed' >&2
    exit 1
fi
chmod 0750 "$ASB_RUNNER_ROOT/runner"
setpriv --reuid "$service_uid" --regid "$service_gid" --clear-groups sh -c 'test ! -r "$ASB_RUNNER_ROOT/control/manifest"'
if setpriv --reuid "$service_uid" --regid "$service_gid" --clear-groups sh -c 'printf poison >> "$ASB_RUNNER_ROOT/runner/bin/Runner.Listener"' 2>/dev/null; then
    echo 'service identity modified immutable installation' >&2
    exit 1
fi
cp "$ASB_RUNNER_ROOT/runner/bin/Runner.Listener" "$ASB_RUNNER_ROOT/control/listener.saved"
printf 'changed\n' >> "$ASB_RUNNER_ROOT/runner/bin/Runner.Listener"
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'drifted installation unexpectedly passed' >&2
    exit 1
fi
mv "$ASB_RUNNER_ROOT/control/listener.saved" "$ASB_RUNNER_ROOT/runner/bin/Runner.Listener"
chown 0:"$service_gid" "$ASB_RUNNER_ROOT/runner/bin/Runner.Listener"
chmod 0550 "$ASB_RUNNER_ROOT/runner/bin/Runner.Listener"

secret='private-host.example token-ghp_fixture /private/session/path'
printf '%s\n' "$secret" > "$ASB_RUNNER_ROOT/runner/_diag/Worker_fixture.log"
chown "$service_uid:$service_gid" "$ASB_RUNNER_ROOT/runner/_diag/Worker_fixture.log"
tools/runners/collect-diagnostics.sh | grep -q 'without names or content'
summary=$ASB_RUNNER_ROOT/control/diagnostics-summary-v1.txt
grep -q '^schema=asb-runner-diagnostics-summary-v1$' "$summary"
grep -q '^files=1$' "$summary"
if grep -F -e private-host -e ghp_fixture -e /private -e Worker_fixture "$summary"; then
    echo 'diagnostic summary leaked private content or names' >&2
    exit 1
fi
rm -f "$ASB_RUNNER_ROOT/runner/_diag/Worker_fixture.log"
dd if=/dev/zero of="$ASB_RUNNER_ROOT/runner/_diag/oversized" bs=65537 count=1 status=none
chown "$service_uid:$service_gid" "$ASB_RUNNER_ROOT/runner/_diag/oversized"
if tools/runners/collect-diagnostics.sh >/dev/null 2>&1; then
    echo 'oversized diagnostics unexpectedly passed' >&2
    exit 1
fi
rm -f "$ASB_RUNNER_ROOT/runner/_diag/oversized"
ln -s /etc/passwd "$ASB_RUNNER_ROOT/runner/_diag/escape"
if tools/runners/collect-diagnostics.sh >/dev/null 2>&1; then
    echo 'symlinked diagnostics unexpectedly passed' >&2
    exit 1
fi
rm -f "$ASB_RUNNER_ROOT/runner/_diag/escape"

exec 7<> "$ASB_RUNNER_ROOT/control/lifecycle.lock"
flock -n 7
if tools/runners/collect-diagnostics.sh >/dev/null 2>&1; then
    echo 'concurrent lifecycle operation unexpectedly passed' >&2
    exit 1
fi
flock -u 7

setpriv --reuid "$service_uid" --regid "$service_gid" --clear-groups sleep 30 &
job_pid=$!
if tools/runners/reset.sh >/dev/null 2>&1; then
    kill "$job_pid" 2>/dev/null || true
    echo 'reset accepted an active service identity' >&2
    exit 1
fi
kill "$job_pid"
wait "$job_pid" 2>/dev/null || true
touch "$ASB_RUNNER_ROOT/runner/_work/residue" "$ASB_RUNNER_ROOT/cache/residue"
mkdir "$ASB_RUNNER_ROOT/.reset-quarantine"
chmod 0700 "$ASB_RUNNER_ROOT/.reset-quarantine"
mkdir "$ASB_RUNNER_ROOT/.reset-quarantine/reset-999-1"
touch "$ASB_RUNNER_ROOT/.reset-quarantine/reset-999-1/interrupted-residue"
rm -rf "$ASB_RUNNER_ROOT/cache"
tools/runners/reset.sh | grep -q '^reset complete$'
test ! -e "$ASB_RUNNER_ROOT/runner/_work/residue" && test ! -e "$ASB_RUNNER_ROOT/cache/residue"
test "$(stat -c '%u:%g:%a' "$ASB_RUNNER_ROOT/runner/_work")" = "$service_uid:$service_gid:700"
test "$(stat -c '%u:%g:%a' "$ASB_RUNNER_ROOT/runner/_diag")" = "$service_uid:$service_gid:700"
for state in .runner .credentials .credentials_rsaparams; do
    test ! -e "$ASB_RUNNER_ROOT/runner/$state"
done
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'reset credentials unexpectedly remained usable' >&2
    exit 1
fi
printf 'fresh-token\n' | tools/runners/register.sh | grep -q 'registered ephemeral runner'
tools/runners/health.sh | grep -q '^healthy version=2.337.0 '

printf 'tester 1\n' > "$ASB_RUNNER_ROOT/control/lease"
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'expired lease unexpectedly passed' >&2
    exit 1
fi
printf 'tester %s\n' "$(( $(date +%s) + 600 ))" > "$ASB_RUNNER_ROOT/control/lease"
chmod 0644 "$ASB_RUNNER_ROOT/control/lease"
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'permissive lease unexpectedly passed' >&2
    exit 1
fi
chmod 0600 "$ASB_RUNNER_ROOT/control/lease"
export ASB_RUNNER_CONFIG_LABELS=self-hosted,linux,x64
if tools/runners/health.sh >/dev/null 2>&1; then
    echo 'generic labels unexpectedly passed' >&2
    exit 1
fi
printf 'runner isolation security fixtures passed\n'
