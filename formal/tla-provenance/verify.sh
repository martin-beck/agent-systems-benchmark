#!/usr/bin/env bash
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

if [[ ${1:-} == --manifest-only && $# == 2 ]]; then
    mode=manifest
    manifest=$2
    source_tree=
    output=
elif [[ $# == 3 ]]; then
    mode=full
    manifest=$1
    source_tree=$2
    output=$3
else
    echo "usage: verify.sh [--manifest-only MANIFEST | MANIFEST SOURCE_TREE OUTPUT]" >&2
    exit 64
fi

python3 - "$mode" "$manifest" "$source_tree" "$output" <<'PY'
import hashlib
import os
from pathlib import Path, PurePosixPath
import re
import stat
import sys
import tomllib
import zipfile

mode, manifest_name, source_name, output_name = sys.argv[1:]

def fail(message: str) -> None:
    raise SystemExit(f"TLA provenance verification failed: {message}")

def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()

def safe_relative(value: str) -> bool:
    path = PurePosixPath(value)
    return bool(value) and not path.is_absolute() and ".." not in path.parts and "\\" not in value

try:
    raw = Path(manifest_name).read_bytes()
    data = tomllib.loads(raw.decode("utf-8"))
except (OSError, UnicodeError, tomllib.TOMLDecodeError):
    fail("manifest is missing, unreadable, or malformed")
if hashlib.sha256(raw).hexdigest() != "83658cff7936f967d5e4feb584cb07b49a89b1a27c872cf0358e7d6350debc91":
    fail("manifest bytes do not match the reviewed contract")

top = {
    "schema_version", "source_repository", "source_commit", "source_archive_url",
    "source_archive_bytes", "source_archive_sha256", "source_license",
    "source_license_path", "source_license_sha256", "ant_version", "ant_archive_url",
    "ant_archive_bytes", "ant_archive_sha256", "ant_archive_sha512", "ant_license_path",
    "ant_license_sha256", "ant_notice_path", "ant_notice_sha256", "ant_license", "container_image",
    "container_architecture", "java_distribution", "java_version", "java_license",
    "java_license_receipts", "container_tool_license_receipts", "network", "environment",
    "build_arguments", "canonical_zip_timestamp", "output_bytes", "output_sha256",
    "output_file_entries", "dependency_count", "dependency",
}
if set(data) != top:
    fail("manifest keys are not the closed schema")
if data["schema_version"] != 1 or data["dependency_count"] != 31:
    fail("unsupported schema version or dependency count")
if data["source_repository"] != "https://github.com/tlaplus/tlaplus":
    fail("source repository is not canonical")
if not re.fullmatch(r"[0-9a-f]{40}", data["source_commit"]):
    fail("source commit is not an exact object id")
if data["source_archive_url"] != f"https://codeload.github.com/tlaplus/tlaplus/tar.gz/{data['source_commit']}":
    fail("source URL is not commit-bound")
if data["network"] != "none" or data["container_architecture"] != "linux/amd64":
    fail("build isolation or architecture changed")
if data["ant_license"] != "Apache-2.0" or data["java_license"] != "GPL-2.0-only WITH Classpath-exception-2.0":
    fail("toolchain SPDX mapping changed")
for receipts, expected_count in [
    (data["java_license_receipts"], 3),
    (data["container_tool_license_receipts"], 4),
]:
    if not isinstance(receipts, list) or len(receipts) != expected_count or len(set(receipts)) != expected_count:
        fail("toolchain license receipt closure changed")
    for receipt in receipts:
        if not re.fullmatch(r"/[^=\n]+=[0-9a-f]{64}", receipt):
            fail("toolchain license receipt malformed")
if data["canonical_zip_timestamp"] != "1980-01-01T00:00:02Z":
    fail("canonical ZIP timestamp changed")
if len(data["environment"]) != 7 or len(set(data["environment"])) != 7:
    fail("environment allowlist changed")
if any(not isinstance(item, str) or "SECRET" in item.upper() or "/home/" in item for item in data["environment"]):
    fail("environment contains private or unbounded data")
if not re.fullmatch(r"[0-9a-f]{64}", data["output_sha256"]):
    fail("output digest malformed")

dependencies = data["dependency"]
if not isinstance(dependencies, list) or len(dependencies) != 31:
    fail("dependency closure is not exactly 31 entries")
expected_dependency_keys = {
    "path", "sha256", "package", "scope", "license", "license_evidence",
    "license_evidence_sha256",
}
license_receipts = {
    "MIT": ("CommunityModules.jar!/LICENSE", "f93a9309b13244337f55615e5b441ff00b124a1cd83c2f55924ba08fb0be42c7"),
    "Apache-2.0": ("cglib-nodep-3.1.jar!/LICENSE", "1eb85fc97224598dad1852b5d6483bbcf0aa8608790dcc657a5a2a761ae9c8c6"),
    "BSD-3-Clause": ("hamcrest-core-1.3.jar!/LICENSE.txt", "1cde867cab5c8e842929da5391cef98b4017314822270e934e8e2eef3767c666"),
    "EPL-1.0": ("junit-4.12.jar!/LICENSE-junit.txt", "9648bb2891b9813970bddb68d4be8a5e6ec8280d0180a53dfb29236b579c55bb"),
    "EPL-2.0": ("javax.mail/mailapi-1.6.8.jar!/META-INF/LICENSE.md", "6e1f002892b81cbe0647019b150c8a056efc1add565671a4f8af629b6cd2cc7b"),
    "EPL-2.0 OR GPL-2.0-only WITH Classpath-exception-2.0": ("javax.mail/mailapi-1.6.8.jar!/META-INF/LICENSE.md", "6e1f002892b81cbe0647019b150c8a056efc1add565671a4f8af629b6cd2cc7b"),
    "GPL-2.0-only WITH Classpath-exception-2.0": ("jmh/jmh-core-1.21.jar!/LICENSE", "4b9abebc4338048a7c2dc184e9f800deb349366bdf28eb23c2677a77b4c87726"),
}
paths = []
for entry in dependencies:
    if set(entry) != expected_dependency_keys:
        fail("dependency keys are not closed")
    if not safe_relative(entry["path"]) or not safe_relative(entry["license_evidence"].split("!/", 1)[0]):
        fail("dependency path escapes the source tree")
    if not re.fullmatch(r"[0-9a-f]{64}", entry["sha256"]):
        fail("dependency digest malformed")
    if entry["scope"] not in {"runtime", "development"}:
        fail("dependency scope unsupported")
    if not entry["license"] or not entry["license_evidence"] or not entry["package"]:
        fail("dependency lacks provenance or license evidence")
    if not re.fullmatch(r"[0-9a-f]{64}", entry["license_evidence_sha256"]):
        fail("license evidence digest malformed")
    if license_receipts.get(entry["license"]) != (entry["license_evidence"], entry["license_evidence_sha256"]):
        fail("license evidence is not an approved actual-text receipt")
    paths.append(entry["path"])
if paths != sorted(paths) or len(paths) != len(set(paths)):
    fail("dependency paths are not sorted and unique")

if mode == "manifest":
    print("TLA source-build manifest is closed: 31 dependencies")
    raise SystemExit(0)

source = Path(source_name)
output = Path(output_name)
if not source.is_dir() or source.is_symlink():
    fail("source tree is not a real directory")
license_path = source / data["source_license_path"]
if not license_path.is_file() or digest(license_path) != data["source_license_sha256"]:
    fail("source license receipt mismatch")
lib = source / "tlatools/org.lamport.tlatools/lib"
actual_jars = sorted(str(path.relative_to(lib)) for path in lib.rglob("*.jar"))
if actual_jars != paths:
    fail("source JAR closure differs from manifest")
for entry in dependencies:
    jar = lib / entry["path"]
    info = jar.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or digest(jar) != entry["sha256"]:
        fail("vendored JAR receipt mismatch")
    evidence = entry["license_evidence"]
    if "!/" in evidence:
        evidence_jar, evidence_entry = evidence.split("!/", 1)
        try:
            with zipfile.ZipFile(lib / evidence_jar) as archive:
                if evidence_entry not in archive.namelist():
                    fail("embedded license evidence missing")
                evidence_bytes = archive.read(evidence_entry)
        except (OSError, zipfile.BadZipFile):
            fail("embedded license evidence unreadable")
    else:
        evidence_path = lib / evidence
        if not evidence_path.is_file():
            fail("source license evidence missing")
        evidence_bytes = evidence_path.read_bytes()
    if hashlib.sha256(evidence_bytes).hexdigest() != entry["license_evidence_sha256"]:
        fail("license evidence digest mismatch")
    lowered = evidence_bytes[:131072].lower()
    if not any(marker in lowered for marker in [b"permission is hereby granted", b"apache license", b"redistribution and use", b"eclipse public license", b"general public license"]):
        fail("license evidence is not an actual license text")

try:
    info = output.lstat()
except OSError:
    fail("output missing")
if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
    fail("output is not a private regular artifact")
if info.st_size != data["output_bytes"] or digest(output) != data["output_sha256"]:
    fail("output size or digest mismatch")
try:
    with zipfile.ZipFile(output) as archive:
        entries = archive.infolist()
        names = [entry.filename for entry in entries if not entry.is_dir()]
        if len(names) != data["output_file_entries"] or len(names) != len(set(names)):
            fail("output entry closure or uniqueness mismatch")
        for entry in entries:
            name = entry.filename.rstrip("/")
            if name and not safe_relative(name):
                fail("unsafe output archive path")
            if entry.date_time != (1980, 1, 1, 0, 0, 2):
                fail("output archive timestamp is not canonical")
            if (entry.external_attr >> 16) & 0o170000 == stat.S_IFLNK:
                fail("output archive contains a symlink")
except (OSError, zipfile.BadZipFile):
    fail("output is not a readable ZIP/JAR")
print("TLA source-build provenance verified: 31 dependencies, 2087 payload files")
PY
