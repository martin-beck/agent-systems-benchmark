# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Fail-closed verifier for the deterministic TLA+ source build."""

import hashlib
import json
import re
import stat
import sys
import zipfile
from pathlib import Path, PurePosixPath

import tomllib

MODE, MANIFEST_NAME, SOURCE_NAME, OUTPUT_NAME, SCRIPT_NAME = sys.argv[1:]


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
    return (
        bool(value)
        and not path.is_absolute()
        and ".." not in path.parts
        and "\\" not in value
    )


BUILD_INPUTS = {
    "gson/gson-2.14.0.jar",
    "javacc-4.0.jar",
    "javax.mail/javax.activation_1.1.0.v201211130549.jar",
    "javax.mail/mailapi-1.6.8.jar",
    "javax.mail/smtp-1.6.8.jar",
    "jline/jline-builtins-3.25.0.jar",
    "jline/jline-console-3.25.0.jar",
    "jline/jline-reader-3.25.0.jar",
    "jline/jline-terminal-3.25.0.jar",
    "lsp/org.eclipse.lsp4j.debug_0.21.1.v20230829-0012.jar",
    "lsp/org.eclipse.lsp4j.jsonrpc.debug_0.21.1.v20230829-0012.jar",
    "lsp/org.eclipse.lsp4j.jsonrpc_0.21.1.v20230829-0012.jar",
    "prettier4j-0.3.2.jar",
}
EXCLUDED = {
    "CommunityModules.jar",
    "cglib-nodep-3.1.jar",
    "easymock-3.3.1.jar",
    "hamcrest-core-1.3.jar",
    "jacocoant.jar",
    "jgit-buildnumber-ant-task-1.2.10.jar",
    "jmh/commons-math3-3.2.jar",
    "jmh/jmh-core-1.21.jar",
    "jmh/jmh-generator-annprocess-1.21.jar",
    "jmh/jopt-simple-4.6.jar",
    "jpf-classes.jar",
    "jpf.jar",
    "jpf/jgraphx.jar",
    "jpf/jpf-shell.jar",
    "jpf/jpf-visual.jar",
    "junit-4.12.jar",
    "objenesis-2.1.jar",
    "org.eclipse.jgit-2.3.1.201302201838-r.jar",
}
LICENSE_EVIDENCE = {
    "gson/gson-2.14.0.jar": (
        "gson/gson-LICENSE",
        "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
    ),
    "javacc-4.0.jar": (
        "@licenses/javacc-4.0-BSD-3-Clause.txt",
        "e53144cce0d644a8da75628f0bf32478eb8b26c2940ec8ea0758d29dd5022efa",
    ),
    "javax.mail/javax.activation_1.1.0.v201211130549.jar": (
        "javax.mail/javax.activation_1.1.0.v201211130549.jar!/META-INF/LICENSE.txt",
        "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
    ),
    "javax.mail/mailapi-1.6.8.jar": (
        "javax.mail/mailapi-1.6.8.jar!/META-INF/LICENSE.md",
        "6e1f002892b81cbe0647019b150c8a056efc1add565671a4f8af629b6cd2cc7b",
    ),
    "javax.mail/smtp-1.6.8.jar": (
        "javax.mail/smtp-1.6.8.jar!/META-INF/LICENSE.md",
        "6e1f002892b81cbe0647019b150c8a056efc1add565671a4f8af629b6cd2cc7b",
    ),
    "jline/jline-builtins-3.25.0.jar": (
        "jline/jline-LICENSE.txt",
        "7d9ca7d4eb3a2534869e7823b81b299321d49ceb3d169d11b995513fabd9fc7e",
    ),
    "jline/jline-console-3.25.0.jar": (
        "jline/jline-LICENSE.txt",
        "7d9ca7d4eb3a2534869e7823b81b299321d49ceb3d169d11b995513fabd9fc7e",
    ),
    "jline/jline-reader-3.25.0.jar": (
        "jline/jline-LICENSE.txt",
        "7d9ca7d4eb3a2534869e7823b81b299321d49ceb3d169d11b995513fabd9fc7e",
    ),
    "jline/jline-terminal-3.25.0.jar": (
        "jline/jline-LICENSE.txt",
        "7d9ca7d4eb3a2534869e7823b81b299321d49ceb3d169d11b995513fabd9fc7e",
    ),
    "lsp/org.eclipse.lsp4j.debug_0.21.1.v20230829-0012.jar": (
        "lsp/org.eclipse.lsp4j.jsonrpc_0.21.1.v20230829-0012.jar!/about.html",
        "9c7157f55d3d48ddecd668cb0c462565f1757071fde71b17d14e821ba2e71828",
    ),
    "lsp/org.eclipse.lsp4j.jsonrpc.debug_0.21.1.v20230829-0012.jar": (
        "lsp/org.eclipse.lsp4j.jsonrpc_0.21.1.v20230829-0012.jar!/about.html",
        "9c7157f55d3d48ddecd668cb0c462565f1757071fde71b17d14e821ba2e71828",
    ),
    "lsp/org.eclipse.lsp4j.jsonrpc_0.21.1.v20230829-0012.jar": (
        "lsp/org.eclipse.lsp4j.jsonrpc_0.21.1.v20230829-0012.jar!/about.html",
        "9c7157f55d3d48ddecd668cb0c462565f1757071fde71b17d14e821ba2e71828",
    ),
    "prettier4j-0.3.2.jar": (
        "@licenses/prettier4j-0.3.2-Apache-2.0.txt",
        "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4",
    ),
}
EXTERNAL_LICENSE_PROVENANCE = {
    "javacc-4.0.jar": {
        "license_source_repository": "https://github.com/javacc/javacc",
        "license_source_ref": "refs/tags/release_40",
        "license_source_commit": "368da68784dca6b29dcf10043f056535b2c835a1",
        "license_source_archive_url": "https://codeload.github.com/javacc/javacc/tar.gz/368da68784dca6b29dcf10043f056535b2c835a1",
        "license_source_archive_bytes": 767184,
        "license_source_archive_sha256": "712420087c0ae91fd221062f0407a47e7abf6478b5ce24b40c7eda509910d27f",
        "license_source_path": "LICENSE",
        "license_source_raw_sha256": "687da1ab44259f7ff84f01d666ad20d7392d599ff2476039581e3bec0417fd2d",
        "license_receipt_transform": "strip_trailing_ascii_whitespace_per_line",
        "license_applicability_path": "build.xml",
        "license_applicability_sha256": "20c2de4787cd1d98cca75072f9191680f15b75174249cc30064365ca0a8b62bb",
    },
    "prettier4j-0.3.2.jar": {
        "license_source_repository": "https://github.com/opencastsoftware/prettier4j",
        "license_source_ref": "refs/tags/v0.3.2",
        "license_source_commit": "48a56fca69a616fa9d555acf4dd0cf6958eb60d0",
        "license_source_archive_url": "https://codeload.github.com/opencastsoftware/prettier4j/tar.gz/48a56fca69a616fa9d555acf4dd0cf6958eb60d0",
        "license_source_archive_bytes": 83041,
        "license_source_archive_sha256": "9f7bf63096ed8974b64b5489886e11831b962768f78ab5844da347d665ff75bd",
        "license_source_path": "LICENSE",
        "license_source_raw_sha256": "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4",
        "license_receipt_transform": "identity",
        "license_applicability_path": "build.gradle.kts",
        "license_applicability_sha256": "d62095cdd0c17f90be6aa6351900f06a21b865b526d09775cb0f3e536ed3e769",
    },
}

try:
    raw = Path(MANIFEST_NAME).read_bytes()
    data = tomllib.loads(raw.decode("utf-8"))
except (OSError, UnicodeError, tomllib.TOMLDecodeError):
    fail("manifest is missing, unreadable, or malformed")

top = {
    "schema_version",
    "source_repository",
    "source_commit",
    "source_archive_url",
    "source_archive_bytes",
    "source_archive_sha256",
    "source_license",
    "source_license_path",
    "source_license_sha256",
    "ant_version",
    "ant_archive_url",
    "ant_archive_bytes",
    "ant_archive_sha256",
    "ant_archive_sha512",
    "ant_license_path",
    "ant_license_sha256",
    "ant_notice_path",
    "ant_notice_sha256",
    "ant_license",
    "container_image",
    "container_architecture",
    "java_distribution",
    "java_version",
    "java_license",
    "java_license_receipts",
    "container_tool_license_receipts",
    "network",
    "environment",
    "build_arguments",
    "canonical_zip_timestamp",
    "output_bytes",
    "output_sha256",
    "output_file_entries",
    "inventory_count",
    "build_input_count",
    "excluded_source_artifact_count",
    "artifact",
}
if set(data) != top:
    fail("manifest keys are not the closed schema")
if (
    data["schema_version"],
    data["inventory_count"],
    data["build_input_count"],
    data["excluded_source_artifact_count"],
) != (1, 31, 13, 18):
    fail("unsupported schema or inventory counts")
if data[
    "source_repository"
] != "https://github.com/tlaplus/tlaplus" or not re.fullmatch(
    r"[0-9a-f]{40}", data["source_commit"]
):
    fail("source identity is not canonical and commit-bound")
if (
    data["source_archive_url"]
    != f"https://codeload.github.com/tlaplus/tlaplus/tar.gz/{data['source_commit']}"
):
    fail("source URL is not commit-bound")
if data["network"] != "none" or data["container_architecture"] != "linux/amd64":
    fail("build isolation or architecture changed")
if (
    data["ant_license"] != "Apache-2.0"
    or data["java_license"] != "GPL-2.0-only WITH Classpath-exception-2.0"
):
    fail("toolchain SPDX mapping changed")
expected_java = [
    "/opt/java/openjdk/legal/java.base/LICENSE=4b9abebc4338048a7c2dc184e9f800deb349366bdf28eb23c2677a77b4c87726",
    "/opt/java/openjdk/legal/java.base/ASSEMBLY_EXCEPTION=a44eb7b5caf5534c6ef536b21edb40b4d6babf91bf97d9d45596868618b2c6fb",
    "/opt/java/openjdk/legal/java.base/ADDITIONAL_LICENSE_INFO=a69bce275ba7a3570af6579cb0f55682cd75fedfcd49e0e8e9022270c447c916",
]
expected_tools = [
    "/usr/share/doc/bash/copyright=da7a8d93abf1eccdeaf326642c8ce9ed760f3a973ca46f3f69b3cf755bb81ade",
    "/usr/share/doc/coreutils/copyright=350c1a60923248396acdf5aa3d20cdd5156e82648b3411bf9dff3a16b1ce9c7e",
    "/usr/share/doc/findutils/copyright=e4ad5b1bb6aa6e7aa93d7e2b516e1ae8de6e75b20cfb4c18934d111d3e918ae4",
    "/usr/share/doc/tar/copyright=5e07fa97dc98e502dc15d09d4c14647ec77509f949267a30d1341cadf84d629f",
]
if (
    data["java_license_receipts"] != expected_java
    or data["container_tool_license_receipts"] != expected_tools
):
    fail("toolchain license receipts are missing, reordered, or unbound")
if (
    data["canonical_zip_timestamp"] != "1980-01-01T00:00:02Z"
    or len(data["environment"]) != 7
    or len(set(data["environment"])) != 7
):
    fail("canonical environment changed")
if any(
    not isinstance(item, str) or "SECRET" in item.upper() or "/home/" in item
    for item in data["environment"]
):
    fail("environment contains private or unbounded data")

artifacts = data["artifact"]
if not isinstance(artifacts, list) or len(artifacts) != 31:
    fail("source inventory is not exactly 31 entries")
paths, build_paths, excluded_paths = [], set(), set()
build_keys = {
    "path",
    "sha256",
    "classification",
    "package",
    "identity_evidence",
    "identity_evidence_sha256",
    "scope",
    "license",
    "license_evidence",
    "license_evidence_sha256",
}
external_license_keys = build_keys | {
    "license_source_repository",
    "license_source_ref",
    "license_source_commit",
    "license_source_archive_url",
    "license_source_archive_bytes",
    "license_source_archive_sha256",
    "license_source_path",
    "license_source_raw_sha256",
    "license_receipt_transform",
    "license_applicability_path",
    "license_applicability_sha256",
}
excluded_keys = {"path", "sha256", "classification"}
for entry in artifacts:
    path = entry.get("path")
    if (
        not isinstance(path, str)
        or not safe_relative(path)
        or not re.fullmatch(r"[0-9a-f]{64}", entry.get("sha256", ""))
    ):
        fail("artifact path or digest malformed")
    paths.append(path)
    if entry.get("classification") == "build_input":
        expected_keys = (
            external_license_keys if path in EXTERNAL_LICENSE_PROVENANCE else build_keys
        )
        if set(entry) != expected_keys or path not in BUILD_INPUTS:
            fail("build-input classification or keys changed")
        if (
            entry["license_evidence"],
            entry["license_evidence_sha256"],
        ) != LICENSE_EVIDENCE[path]:
            fail("build-input package/license applicability changed")
        if (
            entry["identity_evidence"] != "@vendored-jars.json"
            or entry["identity_evidence_sha256"]
            != "eb54f66b56b349d3d65c476ce7e7f12f1b2dcee141f6b1f75cae20cc1d9bb012"
        ):
            fail("build-input coordinate receipt changed")
        expected_provenance = EXTERNAL_LICENSE_PROVENANCE.get(path)
        if expected_provenance is not None and any(
            entry[key] != value for key, value in expected_provenance.items()
        ):
            fail("external license source/applicability receipt changed")
        build_paths.add(path)
    elif entry.get("classification") == "excluded_source_artifact":
        if set(entry) != excluded_keys or path not in EXCLUDED:
            fail("excluded-artifact classification or keys changed")
        excluded_paths.add(path)
    else:
        fail("artifact classification unsupported")
if (
    paths != sorted(paths)
    or len(paths) != len(set(paths))
    or build_paths != BUILD_INPUTS
    or excluded_paths != EXCLUDED
):
    fail("artifact inventory/classification differs from the reviewed partition")
if (
    hashlib.sha256(raw).hexdigest()
    != "16522bf4d56b88d8ec309820b78c5417e4d71d96bfd5a9066859666938ea9aab"
):
    fail("manifest bytes do not match the reviewed contract")
if MODE == "manifest":
    print(
        "TLA source-build manifest is closed: 31 inventoried, 13 build inputs, 18 excluded"
    )
    raise SystemExit(0)

source = Path(SOURCE_NAME)
if not source.is_dir() or source.is_symlink():
    fail("source tree is not a real directory")
lib = source / "tlatools/org.lamport.tlatools/lib"
actual_jars = sorted(str(path.relative_to(lib)) for path in lib.rglob("*.jar"))
expected_actual = sorted(paths if MODE == "inventory" else BUILD_INPUTS)
if actual_jars != expected_actual:
    fail("source JAR inventory differs from the required mode")
license_path = source / data["source_license_path"]
if not license_path.is_file() or digest(license_path) != data["source_license_sha256"]:
    fail("source license receipt mismatch")
by_path = {entry["path"]: entry for entry in artifacts}
for path in actual_jars:
    jar = lib / path
    info = jar.lstat()
    if (
        not stat.S_ISREG(info.st_mode)
        or info.st_nlink != 1
        or digest(jar) != by_path[path]["sha256"]
    ):
        fail("vendored JAR receipt mismatch")

coordinate_file = lib / "vendored-jars.json"
if (
    not coordinate_file.is_file()
    or digest(coordinate_file)
    != "eb54f66b56b349d3d65c476ce7e7f12f1b2dcee141f6b1f75cae20cc1d9bb012"
):
    fail("vendored coordinate receipt mismatch")
try:
    coordinate_map = json.loads(coordinate_file.read_text())["manifests"][
        "vendored-jars"
    ]["resolved"]
except (OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError):
    fail("vendored coordinate receipt malformed")
for path in sorted(BUILD_INPUTS):
    entry = by_path[path]
    key = f"tlatools/org.lamport.tlatools/lib/{path}"
    if coordinate_map.get(key, {}).get("package_url") != entry["package"]:
        fail("build-input coordinate is not bound to its exact source path")
    evidence = entry["license_evidence"]
    if evidence.startswith("@licenses/"):
        evidence_path = Path(SCRIPT_NAME) / evidence[1:]
        if not evidence_path.is_file():
            fail("package-specific license receipt missing")
        evidence_bytes = evidence_path.read_bytes()
    elif "!/" in evidence:
        evidence_jar, evidence_entry = evidence.split("!/", 1)
        try:
            with zipfile.ZipFile(lib / evidence_jar) as archive:
                evidence_bytes = archive.read(evidence_entry)
        except (OSError, KeyError, zipfile.BadZipFile):
            fail("embedded package license receipt unreadable")
    else:
        evidence_path = lib / evidence
        if not evidence_path.is_file():
            fail("source package license receipt missing")
        evidence_bytes = evidence_path.read_bytes()
    if hashlib.sha256(evidence_bytes).hexdigest() != entry["license_evidence_sha256"]:
        fail("package license receipt digest mismatch")
    lowered = evidence_bytes[:262144].lower()
    if not any(
        marker in lowered
        for marker in [
            b"permission is hereby granted",
            b"apache license",
            b"redistribution and use",
            b"eclipse public license",
            b"general public license",
        ]
    ):
        fail("package license receipt is not actual terms")
if MODE in {"inventory", "pruned"}:
    print(f"TLA source inventory verified in {MODE} mode: {len(actual_jars)} JARs")
    raise SystemExit(0)

output = Path(OUTPUT_NAME)
try:
    info = output.lstat()
except OSError:
    fail("output missing")
if (
    not stat.S_ISREG(info.st_mode)
    or info.st_nlink != 1
    or info.st_size != data["output_bytes"]
    or digest(output) != data["output_sha256"]
):
    fail("output identity mismatch")
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
print("TLA source-build provenance verified: 13 build inputs, 2087 payload files")
