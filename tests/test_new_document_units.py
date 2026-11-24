# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The units chosen «for new documents» survive New and a restart (#121),
and never leak into a file that is opened: a document keeps its own."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401

MM = {"length": "mm", "precision": 1}


def test_new_documents_start_in_the_remembered_units(tmp_path, monkeypatch):
    from app_shell.host import Host
    from core import settings, units
    from ingetrazo_core import NativeSettings
    profile = str(tmp_path / "profile")
    monkeypatch.setattr(settings, "_store", NativeSettings(profile))
    units.remember_new_document_units(MM)
    host = Host(0, 900, 600, 1., settings=profile)
    try:
        win = host.window
        scene = win.viewport.scene
        assert scene.units == MM
        assert scene.dimension_style["units"] == "mm"
        assert not win._is_dirty()
        scene.units = {"length": "m", "precision": 2}
        win._on_new()
        assert win.viewport.scene.units == MM
    finally:
        host.close()


def test_an_opened_document_keeps_its_own_units(tmp_path):
    from core import units
    from core.scene import Scene
    from formats import igz
    path = tmp_path / "old.igz"
    igz.save_scene(Scene(), path)                   # drawn in metres
    units.remember_new_document_units(MM)
    try:
        loaded = Scene()
        igz.load_into(loaded, path)
        assert units.model_units_of(loaded)["length"] == "m"
    finally:
        units.remember_new_document_units(units.DEFAULT_MODEL_UNITS)
