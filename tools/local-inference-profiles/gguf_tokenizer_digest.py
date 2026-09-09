# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Hash GGUF tokenizer metadata without reading tensor payloads."""
from __future__ import annotations

import argparse
import hashlib
import struct
from pathlib import Path
from typing import Any, BinaryIO

MAX_KEYS = 128
MAX_ARRAY_ITEMS = 1_000_000
MAX_STRING_BYTES = 16 * 1024 * 1024
MAX_METADATA_BYTES = 64 * 1024 * 1024
TOKENIZER_PREFIX = "tokenizer.ggml."


class GgufError(ValueError):
    """The input is not a bounded GGUF tokenizer metadata stream."""


def _read(stream: BinaryIO, size: int) -> bytes:
    if size < 0 or size > MAX_METADATA_BYTES:
        raise GgufError("field exceeds the metadata bound")
    value = stream.read(size)
    if len(value) != size:
        raise GgufError("truncated GGUF metadata")
    return value


def _u64(stream: BinaryIO) -> int:
    return int(struct.unpack("<Q", _read(stream, 8))[0])


def _string(stream: BinaryIO) -> str:
    size = _u64(stream)
    if size > MAX_STRING_BYTES:
        raise GgufError("GGUF string exceeds the metadata bound")
    try:
        return _read(stream, size).decode("utf-8")
    except UnicodeDecodeError as error:
        raise GgufError("GGUF string is not UTF-8") from error


def _value(stream: BinaryIO, value_type: int) -> Any:
    formats = {
        0: "<B",
        1: "<b",
        2: "<H",
        3: "<h",
        4: "<I",
        5: "<i",
        6: "<f",
        7: "<?",
        10: "<Q",
        11: "<q",
        12: "<d",
    }
    if value_type == 8:
        return _string(stream)
    if value_type == 9:
        element_type = struct.unpack("<I", _read(stream, 4))[0]
        count = _u64(stream)
        if count > MAX_ARRAY_ITEMS:
            raise GgufError("GGUF array exceeds the metadata bound")
        return [_value(stream, element_type) for _ in range(count)]
    if value_type not in formats:
        raise GgufError("unsupported GGUF metadata type")
    return struct.unpack(formats[value_type], _read(stream, struct.calcsize(formats[value_type])))[0]


def _encoded(value: Any) -> bytes:
    if isinstance(value, list):
        result = bytearray(struct.pack("<Q", len(value)))
        for item in value:
            raw = _raw(item)
            result.extend(struct.pack("<Q", len(raw)))
            result.extend(raw)
        return bytes(result)
    raw = _raw(value)
    return struct.pack("<Q", len(raw)) + raw


def _raw(value: Any) -> bytes:
    if isinstance(value, str):
        return value.encode("utf-8")
    if isinstance(value, bool):
        return str(value).encode("ascii")
    return str(value).encode("ascii")


def tokenizer_digest(path: Path) -> str:
    with path.open("rb") as stream:
        if _read(stream, 4) != b"GGUF":
            raise GgufError("GGUF magic is absent")
        version = struct.unpack("<I", _read(stream, 4))[0]
        if version not in {2, 3}:
            raise GgufError("unsupported GGUF version")
        _tensor_count = _u64(stream)
        key_count = _u64(stream)
        if key_count > MAX_KEYS:
            raise GgufError("GGUF metadata key count exceeds the bound")
        selected: dict[str, Any] = {}
        for _ in range(key_count):
            key = _string(stream)
            value = _value(stream, struct.unpack("<I", _read(stream, 4))[0])
            if key.startswith(TOKENIZER_PREFIX):
                selected[key] = value
        digest = hashlib.sha256()
        for key in sorted(selected):
            encoded_key = key.encode("utf-8")
            digest.update(struct.pack("<Q", len(encoded_key)))
            digest.update(encoded_key)
            digest.update(_encoded(selected[key]))
        return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("gguf", type=Path)
    args = parser.parse_args()
    print(tokenizer_digest(args.gguf))


if __name__ == "__main__":
    main()
