# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Keep the ASB/asb-tui application boundary mechanically enforced."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def test_legacy_tui_package_is_not_in_the_asb_tree() -> None:
    """The standalone repository is the only owner of the TUI application."""
    assert not (ROOT / "crates" / "asb-tui").exists()

    workspace = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    assert '"crates/asb-tui"' not in workspace

    lockfile = (ROOT / "Cargo.lock").read_text(encoding="utf-8")
    assert 'name = "asb-tui"' not in lockfile


def test_asb_manifests_have_no_terminal_renderer_dependencies() -> None:
    """Ratatui/Crossterm and terminal renderer crates cannot return to ASB."""
    manifests = list((ROOT / "crates").glob("*/Cargo.toml"))
    forbidden = ("ratatui", "crossterm", "termion", "tui")
    for manifest in manifests:
        contents = manifest.read_text(encoding="utf-8").lower()
        assert not any(name in contents for name in forbidden), manifest


def test_asb_sources_do_not_import_terminal_renderer_crates() -> None:
    """First-party Rust remains free of direct renderer imports."""
    forbidden = ("ratatui", "crossterm", "termion")
    for source in (ROOT / "crates").glob("**/*.rs"):
        contents = source.read_text(encoding="utf-8").lower()
        assert not any(name in contents for name in forbidden), source


def test_cross_repo_qualification_tracks_current_tui_main() -> None:
    """Development qualification must not silently reuse a stale TUI pin."""
    workflow = (ROOT / ".github/workflows/cross-repo-development-qualification.yml").read_text()
    assert 'refs/heads/main' in workflow
    assert 'git -C "$tui_root" fetch --depth=1 origin "$tui_ref"' in workflow
    assert 'rev-parse FETCH_HEAD' in workflow
    assert "8b69a7d9c21a7e3b0a409c7ab525bd998487db63" not in workflow
    assert 'ASB_TUI_SOURCE_COMMIT="$TUI_COMMIT"' in workflow
    assert 'ASB_TUI_SOURCE_TREE="$tui_tree"' in workflow
