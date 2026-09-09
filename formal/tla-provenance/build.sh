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
cp -- "$source_archive" "$ant_archive" "$scratch/"

uid=$(id -u)
gid=$(id -g)
docker_cmd=(docker)
if ! docker info >/dev/null 2>&1; then
    docker_cmd=(sudo -n docker)
fi
"${docker_cmd[@]}" inspect --format '{{.Id}} {{.Os}}/{{.Architecture}}' "$image" | grep -Fx 'sha256:c0d1549d1e0f5fa5b83622ec0033b00456107e0b1d0cfcce4c1d831532ce621e linux/amd64' >/dev/null
"${docker_cmd[@]}" run --rm -i --pull never --network none "$image" sha256sum -c - >/dev/null <<'EOF'
4b9abebc4338048a7c2dc184e9f800deb349366bdf28eb23c2677a77b4c87726  /opt/java/openjdk/legal/java.base/LICENSE
a44eb7b5caf5534c6ef536b21edb40b4d6babf91bf97d9d45596868618b2c6fb  /opt/java/openjdk/legal/java.base/ASSEMBLY_EXCEPTION
a69bce275ba7a3570af6579cb0f55682cd75fedfcd49e0e8e9022270c447c916  /opt/java/openjdk/legal/java.base/ADDITIONAL_LICENSE_INFO
da7a8d93abf1eccdeaf326642c8ce9ed760f3a973ca46f3f69b3cf755bb81ade  /usr/share/doc/bash/copyright
350c1a60923248396acdf5aa3d20cdd5156e82648b3411bf9dff3a16b1ce9c7e  /usr/share/doc/coreutils/copyright
e4ad5b1bb6aa6e7aa93d7e2b516e1ae8de6e75b20cfb4c18934d111d3e918ae4  /usr/share/doc/findutils/copyright
5e07fa97dc98e502dc15d09d4c14647ec77509f949267a30d1341cadf84d629f  /usr/share/doc/tar/copyright
EOF
# The single-quoted script is deliberately expanded only by the container shell.
# shellcheck disable=SC2016
"${docker_cmd[@]}" run --rm --pull never --network none --read-only --user "$uid:$gid" \
    --tmpfs /tmp:rw,nosuid,nodev,noexec,size=256m \
    --mount "type=bind,src=$scratch,dst=/work" --workdir /work "$image" bash -lc '
set -euo pipefail
tar -xzf tlaplus-b123b226.tar.gz
tar -xzf apache-ant-1.10.15-bin.tar.gz
src=$(find . -mindepth 1 -maxdepth 1 -type d -name "tlaplus-*" -print -quit)
test -n "$src"
mkdir -p "$src/tlatools/org.lamport.tlatools/test-class"
env -i PATH=/opt/java/openjdk/bin:/usr/bin:/bin HOME=/tmp TZ=UTC LANG=C.UTF-8 LC_ALL=C.UTF-8 \
    SOURCE_DATE_EPOCH=1788915600 JAVA_HOME=/opt/java/openjdk \
    ./apache-ant-1.10.15/bin/ant -f "$src/tlatools/org.lamport.tlatools/customBuild.xml" \
    -Duser.name=asb-builder -DBUILD_TAG=asb-reproducible \
    -DBuild-Rev=b123b22654942bd7f8b1bcadcc47da4ee2cf4c0e \
    -DTODAY=2026-09-09 -DISO8601=2026-09-09T01:00:00.0Z compile dist >/work/ant.log
cp "$src/tlatools/org.lamport.tlatools/dist/tla2tools.jar" raw.jar
mkdir extracted
(cd extracted && jar --extract --file ../raw.jar)
jar --create --no-manifest --date=1980-01-01T00:00:02Z --file normalized.jar -C extracted .
'
source_tree=$(find "$scratch" -mindepth 1 -maxdepth 1 -type d -name 'tlaplus-*' -print -quit)
"$script_dir/verify.sh" "$manifest" "$source_tree" "$scratch/normalized.jar"
install -m 0644 -- "$scratch/normalized.jar" "$output.partial"
mv -n -- "$output.partial" "$output" || { rm -f -- "$output.partial"; exit 73; }
echo "created deterministic TLA+ artifact"
