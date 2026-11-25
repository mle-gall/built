# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""No third-party vendor files (copyright notice, 2026-09-28): a .skp the
built-in reader cannot open is reported -- no converter runs, nothing is
downloaded -- there is no .skp export, and no bundle carries openskp's
SDK-written blank template."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]


def test_an_unreadable_skp_is_reported_and_nothing_else_runs(native_host, monkeypatch, tmp_path):


    for name in ("_find_skp_converter", "_install_skp_converter",
                 "_extract_skp_dlls", "_on_export_skp"):
        assert not hasattr(native_host.window, name), name
    said = []
    monkeypatch.setattr("built_render.chrome_dialogs.warning",
                        staticmethod(lambda *a, **k: said.append(a[2])))
    monkeypatch.setattr("built_render.chrome_dialogs.question",
                        staticmethod(lambda *a, **k: pytest.fail("asked to install something")))
    # The Render tab looks for Blender when the window opens (one
    # `flatpak info` where Flatpak exists): that is not the .skp import.
    import core.render_blender as rb
    monkeypatch.setattr(rb, "find_blender", lambda *a, **k: None)
    import subprocess
    monkeypatch.setattr(subprocess, "run",
                        lambda *a, **k: pytest.fail("ran an external program"))
    win = native_host.window
    bad = tmp_path / "roto.skp"
    bad.write_bytes(b"not a .skp file at all")
    assert win.import_skp_path(bad) is False
    assert said and "COLLADA" in said[-1]


def test_no_skp_export_in_the_menu(native_host):

    win = native_host.window
    exports = [key for key in win._registry_actions if key.startswith("menu:export:")]
    assert "menu:export:dxf" in exports
    assert "menu:export:skp" not in exports


def test_the_bundle_spec_leaves_the_sdk_template_out():
    text = (ROOT / "packaging/native.spec").read_text(encoding="utf-8")
    assert "'_scaffold' not in src" in text
    flatpak = (ROOT / "packaging/flatpak/com.built.Built.yml").read_text(encoding="utf-8")
    assert "openskp/_scaffold" in flatpak and "rm -rf" in flatpak
    snap = (ROOT / "packaging/snap/snapcraft.yaml.in").read_text(encoding="utf-8")
    assert "_scaffold" in snap
