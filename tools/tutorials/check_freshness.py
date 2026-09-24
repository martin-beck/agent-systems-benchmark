#!/usr/bin/env python3
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Discover and validate every checked-in ASB tutorial without executing it."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

try:
    from .validate import ValidationError, load, validate_document, validate_metadata
except ImportError:  # Script execution from the repository root.
    # mypy checks the package invocation; this branch is only for the CLI script.
    from validate import (  # type: ignore[import-not-found,no-redef]  # pragma: no cover
        ValidationError,
        load,
        validate_document,
        validate_metadata,
    )


TUTORIAL_NAME = re.compile(r"^[a-z0-9][a-z0-9-]*-v1\.json$")
LINK = re.compile(r"\]\(([^)]+)\)")


def tutorial_files(root: Path) -> list[Path]:
    directory = root / "tools" / "tutorials"
    return sorted(
        path
        for path in directory.glob("*-v1.json")
        if TUTORIAL_NAME.fullmatch(path.name)
    )


def _documentation_errors(root: Path, tutorials: list[Path]) -> list[str]:
    docs = sorted((root / "docs").rglob("*.md"))
    text = "\n".join(path.read_text(encoding="utf-8") for path in docs)
    errors: list[str] = []
    for path in tutorials:
        if path.name not in text:
            errors.append(
                f"{path.relative_to(root)} is not referenced by documentation"
            )
    for document in docs:
        for target in LINK.findall(document.read_text(encoding="utf-8")):
            target = target.split("#", 1)[0]
            if "tools/tutorials/" not in target or target.startswith(
                ("http:", "https:")
            ):
                continue
            candidate = (document.parent / target).resolve()
            try:
                candidate.relative_to(root.resolve())
            except ValueError:
                errors.append(
                    f"{document.relative_to(root)} references path outside repository: {target}"
                )
                continue
            if not candidate.is_file():
                errors.append(
                    f"{document.relative_to(root)} references missing tutorial artifact: {target}"
                )
    return errors


def validate_repository(root: Path) -> list[str]:
    metadata_path = root / "tools" / "tutorials" / "command_metadata_v1.json"
    errors: list[str] = []
    try:
        metadata = validate_metadata(load(metadata_path))
    except ValidationError as exc:
        return [f"{metadata_path.relative_to(root)}: {exc}"]
    tutorials = tutorial_files(root)
    if not tutorials:
        return ["tools/tutorials: no versioned tutorial contracts discovered"]
    ids: dict[str, Path] = {}
    for path in tutorials:
        try:
            document = load(path)
            tutorial_id = document.get("tutorial_id")
            if tutorial_id in ids:
                errors.append(
                    f"{path.relative_to(root)}: duplicate tutorial_id {tutorial_id}"
                )
            else:
                ids[tutorial_id] = path
            validate_document(document, metadata)
        except (ValidationError, AttributeError) as exc:
            errors.append(f"{path.relative_to(root)}: {exc}")
    errors.extend(_documentation_errors(root, tutorials))
    return sorted(errors)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", type=Path, default=Path(__file__).parents[2])
    args = parser.parse_args(argv)
    errors = validate_repository(args.root.resolve())
    if errors:
        for error in errors:
            print(f"tutorial freshness failed: {error}", file=sys.stderr)
        return 1
    print(
        f"tutorial freshness: {len(tutorial_files(args.root.resolve()))} contracts validated"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
