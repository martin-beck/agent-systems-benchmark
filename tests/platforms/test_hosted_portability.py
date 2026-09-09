# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
from __future__ import annotations

import copy
import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
TOOLS = ROOT / "tools/platforms"
sys.path.insert(0, str(TOOLS))
SPEC = importlib.util.spec_from_file_location("hosted_portability", TOOLS / "hosted_portability.py")
assert SPEC and SPEC.loader
HOSTED = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = HOSTED
SPEC.loader.exec_module(HOSTED)


def release(version: str) -> dict[str, str]:
    return {"ID": "ubuntu", "VERSION_ID": "24.04", "VERSION": version}


class HostedPortabilityTests(unittest.TestCase):
    def setUp(self) -> None:
        fixture_root = ROOT / "tests/platforms/fixtures/hosted-portability"
        self.report = json.loads((fixture_root / "positive.json").read_text())
        self.mutations = json.loads((fixture_root / "mutations.json").read_text())
        self.schema = json.loads((ROOT / "platforms/v1/hosted-portability.schema.json").read_text())

    def test_closed_schema_positive_and_every_declared_mutation(self) -> None:
        HOSTED._validate_schema(self.report, self.schema, self.schema)
        seen: set[str] = set()
        for mutation in self.mutations:
            self.assertNotIn(mutation["id"], seen)
            seen.add(mutation["id"])
            candidate = copy.deepcopy(self.report)
            target = candidate
            path = mutation["path"]
            for component in path[:-1]:
                target = target[component]
            if "key" in mutation:
                target = candidate
                for component in path:
                    target = target[component]
                target[mutation["key"]] = mutation["value"]
            elif mutation.get("delete"):
                del target[path[-1]]
            else:
                target[path[-1]] = mutation["value"]
            with self.assertRaises(HOSTED.PortabilityError, msg=mutation["id"]):
                HOSTED._validate_schema(candidate, self.schema, self.schema)
        self.assertEqual(len(seen), 17)
        serialized = json.dumps(self.report, sort_keys=True).lower()
        for forbidden in ("hostname", "/home/", "/srv/data/projects", "api_key", "bearer ", "authorization:"):
            self.assertNotIn(forbidden, serialized)

    def test_release_routes_do_not_equate_patch_releases(self) -> None:
        exact = "24.04.4 LTS (Noble Numbat)"
        rolling = "24.04.5 LTS (Noble Numbat)"
        self.assertEqual(HOSTED.release_route(release(exact)), "native-qualification")
        self.assertEqual(HOSTED.release_route(release(rolling)), "hosted-portability")
        with self.assertRaisesRegex(HOSTED.native.EvidenceError, "exact pinned release"):
            HOSTED.native.validate_platform("ubuntu-24.04", release(rolling), Path("/"))
        for malformed in (
            {"ID": "debian", "VERSION_ID": "24.04", "VERSION": rolling},
            release("24.04.3 LTS (Noble Numbat)"),
            release("24.04.5"),
            release("24.04.5 LTS (Other)"),
        ):
            with self.assertRaises(HOSTED.PortabilityError):
                HOSTED.release_route(malformed)

    def test_collect_is_bound_to_rolling_release_source_and_exact_checks(self) -> None:
        check = {
            "argv_sha256": "sha256:" + "a" * 64,
            "output_bytes": 0,
            "output_sha256": "sha256:" + "b" * 64,
            "status": "passed",
        }
        checks = [(name, ["/usr/bin/true"]) for name in ("process", "metrics", "sandbox")]
        os_release = "ID=ubuntu\nVERSION_ID=\"24.04\"\nVERSION=\"24.04.5 LTS (Noble Numbat)\"\n"
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            root = Path(temporary)
            (root / "etc").mkdir()
            (root / "etc/os-release").write_text(os_release)
            with (
                mock.patch.object(HOSTED.platform, "machine", return_value="x86_64"),
                mock.patch.object(HOSTED, "_source", side_effect=[("d" * 40, "e" * 40), ("d" * 40, "e" * 40)]),
                mock.patch.object(HOSTED, "_run", return_value=check),
            ):
                report = HOSTED.collect("ubuntu-24.04", "x86_64", "gha-123-1", ROOT, "c" * 40, checks, root)
            HOSTED._validate_schema(report, self.schema, self.schema)
            self.assertFalse(report["observed"]["distribution"]["exact_native_release"])
            with self.assertRaisesRegex(HOSTED.PortabilityError, "exactly"):
                HOSTED.collect("ubuntu-24.04", "x86_64", "gha-123-1", ROOT, "c" * 40, checks[:-1], root)

    def test_failures_redact_raw_check_diagnostics_and_source_races(self) -> None:
        with mock.patch.object(
            HOSTED.native,
            "run_check",
            side_effect=HOSTED.native.EvidenceError("PRIVATE_SENTINEL"),
        ), self.assertRaises(HOSTED.PortabilityError) as caught:
            HOSTED._run(["/usr/bin/false"], ROOT)
        self.assertNotIn("PRIVATE_SENTINEL", str(caught.exception))

        checks = [(name, ["/usr/bin/true"]) for name in ("process", "metrics", "sandbox")]
        os_release = "ID=ubuntu\nVERSION_ID=24.04\nVERSION=\"24.04.5 LTS (Noble Numbat)\"\n"
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            root = Path(temporary)
            (root / "etc").mkdir()
            (root / "etc/os-release").write_text(os_release)
            with (
                mock.patch.object(HOSTED.platform, "machine", return_value="x86_64"),
                mock.patch.object(
                    HOSTED,
                    "_source",
                    side_effect=[("d" * 40, "e" * 40), ("f" * 40, "e" * 40)],
                ),
                mock.patch.object(HOSTED, "_run", return_value=self.report["checks"]["process"]),
                self.assertRaisesRegex(HOSTED.PortabilityError, "changed"),
            ):
                HOSTED.collect(
                    "ubuntu-24.04", "x86_64", "gha-123-1", ROOT, "c" * 40, checks, root
                )

    def test_atomic_output_rejects_stale_file_and_symlinked_parent(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            root = Path(temporary)
            output = root / "hosted.json"
            HOSTED.native.write_atomic(output, self.report, root)
            with self.assertRaisesRegex(HOSTED.native.EvidenceError, "already exist"):
                HOSTED.native.write_atomic(output, self.report, root)
            target = root / "target"
            target.mkdir()
            link = root / "link"
            link.symlink_to(target, target_is_directory=True)
            with self.assertRaisesRegex(HOSTED.native.EvidenceError, "symlink"):
                HOSTED.native.write_atomic(link / "other.json", self.report, root)

    def test_workflow_keeps_artifact_kinds_distinct_and_conditional(self) -> None:
        workflow = (ROOT / ".github/workflows/native-platforms.yml").read_text()
        self.assertIn("hosted_portability.py route", workflow)
        self.assertIn("hosted_portability.py collect", workflow)
        self.assertIn("native_evidence.py", workflow)
        self.assertIn("hosted_portability.py validate-artifact", workflow)
        self.assertIn("steps.evidence.outputs.kind == 'hosted-portability'", workflow)
        self.assertIn("steps.evidence.outputs.kind == 'native-qualification'", workflow)
        self.assertIn("hosted-portability-${{ matrix.runner }}", workflow)
        self.assertIn("native-qualification-${{ matrix.runner }}", workflow)
        self.assertNotIn("continue-on-error", workflow)
        self.assertNotIn("name: Native platform evidence", workflow)
        self.assertNotIn("name: Native Ubuntu", workflow)

    def test_executable_routing_validates_schema_and_cleans_every_failure(self) -> None:
        tool = TOOLS / "hosted_portability.py"
        native_fixtures = sorted(
            (ROOT / "platforms/v1/native-evidence").glob(
                "ubuntu-24.04-x86_64-native-*.json"
            )
        )
        self.assertEqual(len(native_fixtures), 1)
        native_fixture = native_fixtures[0]
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            output_root = Path(temporary)
            hosted = output_root / "hosted.json"
            native_file = output_root / "native.json"

            hosted.write_text(json.dumps(self.report))
            success = subprocess.run(
                [
                    sys.executable, str(tool), "validate-artifact", "--route",
                    "hosted-portability", "--native-file", str(native_file),
                    "--hosted-file", str(hosted), "--output-root", str(output_root),
                    "--source", str(ROOT),
                ],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(success.returncode, 0)
            self.assertEqual(success.stdout.strip(), "hosted-portability")
            self.assertTrue(hosted.is_file())

            malformed = copy.deepcopy(self.report)
            malformed["qualification"] = "native-functional"
            hosted.unlink()
            hosted.write_text(json.dumps(malformed))
            rejected = subprocess.run(
                [
                    sys.executable, str(tool), "validate-artifact", "--route",
                    "hosted-portability", "--native-file", str(native_file),
                    "--hosted-file", str(hosted), "--output-root", str(output_root),
                    "--source", str(ROOT),
                ],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(rejected.returncode, 1)
            self.assertEqual(
                rejected.stdout.strip(),
                "ERROR: platform evidence does not match its closed schema",
            )
            self.assertEqual(rejected.stderr, "")
            self.assertFalse(hosted.exists())
            self.assertFalse(native_file.exists())

            shutil.copyfile(native_fixture, native_file)
            native_success = subprocess.run(
                [
                    sys.executable, str(tool), "validate-artifact", "--route",
                    "native-qualification", "--native-file", str(native_file),
                    "--hosted-file", str(hosted), "--output-root", str(output_root),
                    "--source", str(ROOT),
                ],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(native_success.returncode, 0)
            self.assertEqual(native_success.stdout.strip(), "native-qualification")
            self.assertEqual(native_success.stderr, "")
            self.assertTrue(native_file.is_file())
            native_file.unlink()

            hosted.write_text(json.dumps(self.report))
            shutil.copyfile(native_fixture, native_file)
            ambiguous = subprocess.run(
                [
                    sys.executable, str(tool), "validate-artifact", "--route",
                    "native-qualification", "--native-file", str(native_file),
                    "--hosted-file", str(hosted), "--output-root", str(output_root),
                    "--source", str(ROOT),
                ],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(ambiguous.returncode, 1)
            self.assertEqual(
                ambiguous.stdout.strip(),
                "ERROR: more than one platform evidence kind exists",
            )
            self.assertEqual(ambiguous.stderr, "")
            self.assertFalse(hosted.exists())
            self.assertFalse(native_file.exists())

            hosted.write_text(json.dumps(self.report))
            invalid_route = subprocess.run(
                [
                    sys.executable, str(tool), "validate-artifact", "--route",
                    "unknown", "--native-file", str(native_file),
                    "--hosted-file", str(hosted), "--output-root", str(output_root),
                    "--source", str(ROOT),
                ],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(invalid_route.returncode, 1)
            self.assertEqual(
                invalid_route.stdout.strip(), "ERROR: platform evidence route is invalid"
            )
            self.assertEqual(invalid_route.stderr, "")
            self.assertFalse(hosted.exists())

    def test_validator_runs_without_site_packages_or_ambient_dependencies(self) -> None:
        tool = TOOLS / "hosted_portability.py"
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            output_root = Path(temporary)
            hosted = output_root / "hosted.json"
            native_file = output_root / "native.json"
            hosted.write_text(json.dumps(self.report))
            result = subprocess.run(
                [
                    sys.executable, "-S", str(tool), "validate-artifact", "--route",
                    "hosted-portability", "--native-file", str(native_file),
                    "--hosted-file", str(hosted), "--output-root", str(output_root),
                    "--source", str(ROOT),
                ],
                capture_output=True, text=True, check=False,
                env={"PATH": "/usr/bin:/bin", "PYTHONNOUSERSITE": "1"},
            )
            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertEqual(result.stdout.strip(), "hosted-portability")
            self.assertEqual(result.stderr, "")
            self.assertTrue(hosted.is_file())
            self.assertFalse(native_file.exists())

            source = output_root / "source"
            schemas = source / "platforms/v1"
            schemas.mkdir(parents=True)
            schema_path = schemas / "hosted-portability.schema.json"
            schema_path.write_bytes(
                (ROOT / "platforms/v1/hosted-portability.schema.json").read_bytes() + b"\n"
            )
            hosted.unlink()
            hosted.write_text(json.dumps(self.report))
            altered_schema = subprocess.run(
                [
                    sys.executable, "-S", str(tool), "validate-artifact", "--route",
                    "hosted-portability", "--native-file", str(native_file),
                    "--hosted-file", str(hosted), "--output-root", str(output_root),
                    "--source", str(source),
                ],
                capture_output=True, text=True, check=False,
                env={"PATH": "/usr/bin:/bin", "PYTHONNOUSERSITE": "1"},
            )
            self.assertEqual(altered_schema.returncode, 1)
            self.assertEqual(
                altered_schema.stdout.strip(),
                "ERROR: platform evidence schema identity is not exact",
            )
            self.assertEqual(altered_schema.stderr, "")
            self.assertFalse(hosted.exists())


if __name__ == "__main__":
    unittest.main()
