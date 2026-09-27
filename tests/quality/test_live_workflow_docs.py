# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT

"""Keep the public benchmark workflow docs aligned with the CLI contract."""

from pathlib import Path


ROOT = Path(__file__).parents[2]


def test_workflow_docs_describe_current_cli_and_optional_live_boundary() -> None:
    readme = (ROOT / "README.md").read_text()
    quickstart = (ROOT / "docs/QUICKSTART.md").read_text()
    workflow = (ROOT / "docs/workflows/live-benchmark.md").read_text()
    provider = (ROOT / "docs/PROVIDER_LAUNCH.md").read_text()

    assert "Provider recording/replay and live-provider selection are not CLI" not in readme
    assert "There is no `asb record` or `asb replay` command" not in readme
    for command in ("provider-catalog", "provider-plan", "record-live", "replay-offline"):
        assert command in readme
        assert command in quickstart or command in workflow
    assert "e834d287378fc7b1b19cd93cf5fa28fc2c753d80a6da05f8e3d2ac45b1af56c7" in provider
    assert "catalog-selectable" in provider
    assert "Gemini is rejected before launch" in provider
    assert "provider reachability is optional" in workflow
    assert "no live-provider fallback" in workflow
