# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Independently validate the experiment-v1 content-address reference fixture."""

import copy
import hashlib
import json
import struct
import sys
from pathlib import Path


def text(value: str) -> bytes:
    encoded = value.encode("utf-8")
    return struct.pack(">Q", len(encoded)) + encoded


def integer(value: int, width: str) -> bytes:
    return struct.pack(">" + width, value)


def optional(value, encode) -> bytes:
    return b"\x00" if value is None else b"\x01" + encode(value)


def experiment_digest(document: dict) -> str:
    assert document["version"] == "1"
    agent = document["agent"]
    model = document["model"]
    settings = model["settings"]
    policy = document["tool_policy"]
    workload = document["workload"]
    execution = document["execution"]
    platform = document["platform"]
    controls = document["controls"]
    retry = controls["retry"]
    replay = controls["replay"]

    fields = [
        b"asb-experiment-v1\x00",
        text(agent["implementation"]),
        text(agent["revision"]),
        text(agent["binary_sha256"]),
        text(model["provider"]),
        text(model["model"]),
        optional(settings["temperature_milli"], lambda value: integer(value, "H")),
        optional(settings["top_p_millionth"], lambda value: integer(value, "I")),
        optional(settings["seed"], lambda value: integer(value, "Q")),
        optional(settings["max_output_tokens"], lambda value: integer(value, "I")),
        optional(settings["reasoning_effort"], text),
        optional(settings["additional_settings_sha256"], text),
        text(policy["policy"]),
        text(policy["revision"]),
        text(policy["content_sha256"]),
        text(workload["workload"]),
        text(workload["workload_revision"]),
        text(workload["workload_sha256"]),
        text(workload["scorer_revision"]),
        text(workload["scorer_sha256"]),
        text(execution["image_sha256"]),
        text(execution["dependencies_sha256"]),
        text(platform["kernel_release"]),
        text(platform["distribution"]),
        text(platform["distribution_version"]),
        text(platform["architecture"]),
        text(platform["cpu_model"]),
        integer(platform["logical_cpu_count"], "I"),
        integer(platform["numa_node_count"], "I"),
        text(platform["scaling_governor"]),
        bytes([{"cold": 0, "warm": 1, "uncontrolled": 2}[controls["cache_state"]]]),
        text(controls["load_policy"]),
        integer(retry["limit"], "H"),
        text(retry["strategy"]),
        integer(retry["backoff_ms"], "Q"),
        bytes([{"live": 0, "replay": 1}[replay["mode"]]]),
        optional(replay["cassette_sha256"], text),
        text(replay["pacing"]),
        integer(replay["cancellation_timeout_ms"], "Q"),
    ]
    return hashlib.sha256(b"".join(fields)).hexdigest()


fixture_name = (
    sys.argv[2]
    if len(sys.argv) == 3 and sys.argv[1] == "--digest"
    else "experiment-manifest.json"
)
fixture_path = Path(__file__).parents[1] / "fixtures/v1" / fixture_name
fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
actual = experiment_digest(fixture)
if sys.argv[1:2] == ["--digest"]:
    print(actual)
    raise SystemExit(0)

assert fixture["experiment_sha256"] == actual
if fixture_name == "experiment-manifest.json":
    confounded_path = fixture_path.with_name("experiment-manifest-confounded.json")
    confounded = json.loads(confounded_path.read_text(encoding="utf-8"))
    assert confounded["experiment_sha256"] == experiment_digest(confounded)

# Retained negative mutants prove independently that the address binds both a
# platform confounder and a model setting.
governor_mutant = copy.deepcopy(fixture)
governor_mutant["platform"]["scaling_governor"] = "powersave"
assert experiment_digest(governor_mutant) != actual
model_mutant = copy.deepcopy(fixture)
model_mutant["model"]["settings"]["seed"] += 1
assert experiment_digest(model_mutant) != actual

print(f"validated experiment fixture {actual}")
