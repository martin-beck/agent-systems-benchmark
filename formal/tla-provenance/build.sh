#!/usr/bin/env bash
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

if [[ $# != 2 ]]; then
    echo "usage: build.sh CACHE_DIR OUTPUT_JAR" >&2
    exit 64
fi

cache=$1
output=$2
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
manifest=$script_dir/source-build.toml
source_archive=$cache/tlaplus-b123b226.tar.gz
ant_archive=$cache/apache-ant-1.10.15-bin.tar.gz
image=eclipse-temurin@sha256:c0d1549d1e0f5fa5b83622ec0033b00456107e0b1d0cfcce4c1d831532ce621e

[[ -f $source_archive && ! -L $source_archive ]] || { echo "source archive missing" >&2; exit 65; }
[[ -f $ant_archive && ! -L $ant_archive ]] || { echo "Ant archive missing" >&2; exit 65; }
printf '%s  %s\n' 1f96ee7ef950e456794d13b7e4d8123c345a91528257a3a41b7cc1b506d1b58f "$source_archive" | sha256sum -c - >/dev/null
printf '%s  %s\n' 71334d7e5d98cfe53d6c429a648a5021137a967378667306c5f613dff5180506 "$ant_archive" | sha256sum -c - >/dev/null
[[ $(stat -c %s -- "$source_archive") == 82989507 && $(stat -c %s -- "$ant_archive") == 6925830 ]] || { echo "archive size mismatch" >&2; exit 65; }
python3 - "$source_archive" "$ant_archive" <<'PY'
from pathlib import PurePosixPath
import sys
import tarfile

for archive_name in sys.argv[1:]:
    seen = set()
    total = 0
    with tarfile.open(archive_name, "r:gz") as archive:
        members = archive.getmembers()
        if len(members) > 100_000:
            raise SystemExit("archive member limit exceeded")
        for member in members:
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or "\\" in member.name:
                raise SystemExit("unsafe archive path")
            if member.name in seen:
                raise SystemExit("duplicate archive member")
            seen.add(member.name)
            if not (member.isfile() or member.isdir()):
                raise SystemExit("archive contains a link or special file")
            total += member.size
            if total > 512 * 1024 * 1024:
                raise SystemExit("archive expansion limit exceeded")
with tarfile.open(sys.argv[2], "r:gz") as ant:
    for name, expected in {
        "apache-ant-1.10.15/LICENSE": "6d62d1dd52f932865d6f1e28920d53d0a6f9730ef6b8e02c48d99f470e160a0d",
        "apache-ant-1.10.15/NOTICE": "9bb3ffffe75d4d79d4e0cdd173050fcf93db0acccfa16af55cd10405d7523218",
    }.items():
        stream = ant.extractfile(name)
        if stream is None or __import__("hashlib").sha256(stream.read()).hexdigest() != expected:
            raise SystemExit("Ant license receipt mismatch")
PY
[[ ! -e $output ]] || { echo "output already exists" >&2; exit 73; }
mkdir -p -- "$(dirname -- "$output")"
lock=$output.lock
mkdir -- "$lock" 2>/dev/null || { echo "another build owns the output" >&2; exit 75; }
scratch=$(mktemp -d "${TMPDIR:-/tmp}/asb-tla-build.XXXXXXXX")
chmod 700 "$scratch"
cleanup() { rm -rf -- "$scratch"; rm -f -- "$output.partial"; rmdir -- "$lock" 2>/dev/null || true; }
cancel() { trap - EXIT HUP INT TERM; cleanup; exit 130; }
trap cleanup EXIT
trap cancel HUP INT TERM
cp -- "$ant_archive" "$scratch/"

uid=$(id -u)
gid=$(id -g)
docker_cmd=(docker)
if ! docker info >/dev/null 2>&1; then
    docker_cmd=(sudo -n docker)
fi
if ! "${docker_cmd[@]}" image inspect --format \
    '{"config_id":{{json .Id}},"repo_digests":{{json .RepoDigests}},"os":{{json .Os}},"architecture":{{json .Architecture}}}' \
    "$image" 2>/dev/null \
    | "$script_dir/verify_image_identity.sh" "$image" linux amd64; then
    echo "OCI image inspection or verification failed" >&2
    exit 65
fi
"${docker_cmd[@]}" run --rm -i --pull never --network none "$image" sha256sum -c - >/dev/null <<'EOF'
4b9abebc4338048a7c2dc184e9f800deb349366bdf28eb23c2677a77b4c87726  /opt/java/openjdk/legal/java.base/LICENSE
a44eb7b5caf5534c6ef536b21edb40b4d6babf91bf97d9d45596868618b2c6fb  /opt/java/openjdk/legal/java.base/ASSEMBLY_EXCEPTION
a69bce275ba7a3570af6579cb0f55682cd75fedfcd49e0e8e9022270c447c916  /opt/java/openjdk/legal/java.base/ADDITIONAL_LICENSE_INFO
da7a8d93abf1eccdeaf326642c8ce9ed760f3a973ca46f3f69b3cf755bb81ade  /usr/share/doc/bash/copyright
350c1a60923248396acdf5aa3d20cdd5156e82648b3411bf9dff3a16b1ce9c7e  /usr/share/doc/coreutils/copyright
e4ad5b1bb6aa6e7aa93d7e2b516e1ae8de6e75b20cfb4c18934d111d3e918ae4  /usr/share/doc/findutils/copyright
5e07fa97dc98e502dc15d09d4c14647ec77509f949267a30d1341cadf84d629f  /usr/share/doc/tar/copyright
EOF
tar -xzf "$ant_archive" -C "$scratch"

excluded=(
    CommunityModules.jar cglib-nodep-3.1.jar easymock-3.3.1.jar
    hamcrest-core-1.3.jar jacocoant.jar jgit-buildnumber-ant-task-1.2.10.jar
    jmh/commons-math3-3.2.jar jmh/jmh-core-1.21.jar
    jmh/jmh-generator-annprocess-1.21.jar jmh/jopt-simple-4.6.jar
    jpf-classes.jar jpf.jar jpf/jgraphx.jar jpf/jpf-shell.jar
    jpf/jpf-visual.jar junit-4.12.jar objenesis-2.1.jar
    org.eclipse.jgit-2.3.1.201302201838-r.jar
)
retained=(
    gson/gson-2.14.0.jar javacc-4.0.jar
    javax.mail/javax.activation_1.1.0.v201211130549.jar
    javax.mail/mailapi-1.6.8.jar javax.mail/smtp-1.6.8.jar
    jline/jline-builtins-3.25.0.jar jline/jline-console-3.25.0.jar
    jline/jline-reader-3.25.0.jar jline/jline-terminal-3.25.0.jar
    lsp/org.eclipse.lsp4j.debug_0.21.1.v20230829-0012.jar
    lsp/org.eclipse.lsp4j.jsonrpc.debug_0.21.1.v20230829-0012.jar
    lsp/org.eclipse.lsp4j.jsonrpc_0.21.1.v20230829-0012.jar
    prettier4j-0.3.2.jar
)

prepare_source() {
    local destination=$1 library artifact
    mkdir -- "$destination"
    tar -xzf "$source_archive" --strip-components=1 -C "$destination"
    "$script_dir/verify.sh" --inventory "$manifest" "$destination"
    library=$destination/tlatools/org.lamport.tlatools/lib
    for artifact in "${excluded[@]}"; do
        rm -- "$library/$artifact"
    done
    "$script_dir/verify.sh" --pruned "$manifest" "$destination"
}

# Run only against a tree which has already passed the exact post-prune allowlist.
run_ant() {
    local source_relative=$1 normalized_relative=$2 log_relative=$3
    # The single-quoted script is deliberately expanded only by the container shell.
    # shellcheck disable=SC2016
    "${docker_cmd[@]}" run --rm --pull never --network none --read-only --user "$uid:$gid" \
        --tmpfs /tmp:rw,nosuid,nodev,noexec,size=256m \
        --mount "type=bind,src=$scratch,dst=/work" --workdir /work "$image" \
        bash -c '
set -euo pipefail
src=$1
normalized=$2
log=$3
mkdir -p "$src/tlatools/org.lamport.tlatools/test-class"
env -i PATH=/opt/java/openjdk/bin:/usr/bin:/bin HOME=/tmp TZ=UTC LANG=C.UTF-8 LC_ALL=C.UTF-8 \
    SOURCE_DATE_EPOCH=1788915600 JAVA_HOME=/opt/java/openjdk \
    /work/apache-ant-1.10.15/bin/ant -f "$src/tlatools/org.lamport.tlatools/customBuild.xml" \
    -Duser.name=asb-builder -DBUILD_TAG=asb-reproducible \
    -DBuild-Rev=b123b22654942bd7f8b1bcadcc47da4ee2cf4c0e \
    -DTODAY=2026-09-09 -DISO8601=2026-09-09T01:00:00.0Z compile dist >"$log"
raw="$src/tlatools/org.lamport.tlatools/dist/tla2tools.jar"
extract="$src/.asb-normalized"
mkdir "$extract"
(cd "$extract" && jar --extract --file "$raw")
jar --create --no-manifest --date=1980-01-01T00:00:02Z --file "$normalized" -C "$extract" .
' -- "$source_relative" "$normalized_relative" "$log_relative"
}

prepare_source "$scratch/run-one"
prepare_source "$scratch/run-two"

# A retained input is accepted only when deleting it prevents reproduction of the
# reviewed output. Each trial starts from the original archive and is pruned before
# its first Ant invocation.
for index in "${!retained[@]}"; do
    necessity=$scratch/necessity-$index
    prepare_source "$necessity"
    rm -- "$necessity/tlatools/org.lamport.tlatools/lib/${retained[$index]}"
    if run_ant "/work/necessity-$index" "/work/necessity-$index.jar" "/work/necessity-$index.log"; then
        if [[ $(sha256sum "$scratch/necessity-$index.jar" | cut -d' ' -f1) == 8c200a88d151c6c183c8dbc57a6b633d135e7a2b18242a3afbf243a9e4b68d3e ]]; then
            echo "retained input is not necessary: ${retained[$index]}" >&2
            exit 65
        fi
    fi
done

run_ant /work/run-one /work/run-one.jar /work/run-one.log
run_ant /work/run-two /work/run-two.jar /work/run-two.log
cmp --silent -- "$scratch/run-one.jar" "$scratch/run-two.jar" || { echo "repeated builds differ" >&2; exit 65; }
"$script_dir/verify.sh" "$manifest" "$scratch/run-one" "$scratch/run-one.jar"
"$script_dir/verify.sh" "$manifest" "$scratch/run-two" "$scratch/run-two.jar"
install -m 0644 -- "$scratch/run-one.jar" "$output.partial"
mv -n -- "$output.partial" "$output" || { rm -f -- "$output.partial"; exit 73; }
echo "created deterministic TLA+ artifact"
