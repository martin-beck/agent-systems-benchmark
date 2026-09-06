# SPDX-License-Identifier: MIT
"""Independently validate the provider-profile-v1 reference address."""

import copy
import hashlib
import json
import struct
from pathlib import Path


def text(value: str) -> bytes:
    encoded = value.encode("utf-8")
    return struct.pack(">Q", len(encoded)) + encoded


def integer(value: int, width: str) -> bytes:
    return struct.pack(">" + width, value)


def optional(value, encode) -> bytes:
    return b"\x00" if value is None else b"\x01" + encode(value)


def provider_digest(document: dict) -> str:
    version = document["version"]
    endpoint = document["endpoint"]
    settings = document["settings"]
    transport = document["transport"]
    credential = document["credential"]
    provider_tags = {
        "open_ai": 0,
        "open_ai_compatible": 1,
        "anthropic": 2,
        "ollama": 3,
    }
    endpoint_tags = {
        "public_service": 0,
        "loopback": 1,
        "private_network": 2,
        "replay": 3,
    }
    credential_tags = {
        "none": 0,
        "environment": 1,
        "file_descriptor": 2,
        "helper": 3,
    }
    fields = [
        b"asb-provider-profile-v1\x00",
        integer(version["major"], "H"),
        integer(version["minor"], "H"),
        bytes([provider_tags[document["provider"]]]),
        bytes([endpoint_tags[endpoint["class"]]]),
        text(endpoint["identity_sha256"]),
        text(document["model"]),
        optional(settings["temperature_milli"], lambda value: integer(value, "H")),
        optional(settings["top_p_millionth"], lambda value: integer(value, "I")),
        optional(settings["seed"], lambda value: integer(value, "Q")),
        optional(settings["max_output_tokens"], lambda value: integer(value, "I")),
        optional(settings["reasoning_effort"], text),
        optional(settings["additional_settings_sha256"], text),
        integer(transport["max_request_bytes"], "I"),
        integer(transport["max_response_bytes"], "I"),
        integer(transport["connect_timeout_ms"], "Q"),
        integer(transport["request_timeout_ms"], "Q"),
        integer(transport["max_concurrent_requests"], "H"),
        bytes([credential_tags[credential["source"]]]),
        optional(credential["reference_sha256"], text),
    ]
    return hashlib.sha256(b"".join(fields)).hexdigest()


fixture_path = Path(__file__).parents[1] / "fixtures/v1/provider-profile.json"
fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
actual = provider_digest(fixture)
assert fixture["settings_sha256"] == actual

model_mutant = copy.deepcopy(fixture)
model_mutant["model"] = "other-model"
assert provider_digest(model_mutant) != actual
omission_mutant = copy.deepcopy(fixture)
omission_mutant["settings"]["seed"] = None
assert provider_digest(omission_mutant) != actual
credential_mutant = copy.deepcopy(fixture)
credential_mutant["credential"]["reference_sha256"] = "b" * 64
assert provider_digest(credential_mutant) != actual

print(f"validated provider fixture {actual}")
