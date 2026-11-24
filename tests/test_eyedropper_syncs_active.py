# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The eyedropper refreshes the Materials panel's «Activo» swatch.

After sampling a face (Alt, or the toolbar pipette) the swatch and the
texture size fields must say what the next click paints (issue #47). The
call went to the tray dock, which has no ``sync_from_paint`` — the method
lives on its Materials panel — so nothing ever refreshed."""
from __future__ import annotations

from core.input_types import QPointF, Qt
from ingetrazo_core import Vec3 as QVector3D
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_tree_links import _View
from tools.base import ToolContext
from tools.paint import PaintTool
from views import wgpu_library, wgpu_ui


def _sample(win, face, monkeypatch):
    vp = win.viewport
    monkeypatch.setattr(vp, "pick_face_any", lambda *_a: (face, None))
    monkeypatch.setattr(vp, "_pixel_to_ray", lambda *_a: (None, None))
    PaintTool().on_click(ToolContext(viewport=vp, world=QVector3D(),
                                     screen=QPointF(0, 0),
                                     modifiers=Qt.AltModifier, snap=None))


def _quad(win):
    return win.viewport.scene.mesh.add_face(
        [QVector3D(0, 0, 0), QVector3D(4, 0, 0),
         QVector3D(4, 4, 0), QVector3D(0, 4, 0)])


def shown(view, win):
    wgpu_ui._inspector(view, win)
    title, sections = view.renderer.calls["ui_set_inspector"]
    assert title == "Active paint"
    return {k: v.rsplit("\x1d", 1)[-1] for k, v in dict(sections)["Next paint stroke"]}


def test_sampled_colour_shows_in_the_active_swatch(native_host, monkeypatch):
    win, view = native_host.window, _View()
    win._activate_tool("paint")
    face = _quad(win)
    for rgb, expected in [((.8, .4, .2), "#cc6633"), ((.1, .6, .2), "#1a9933")]:
        face.attrs["color"] = rgb
        _sample(win, face, monkeypatch)
        assert shown(view, win)["Colour"] == expected


def test_sampled_texture_loads_the_size_fields(native_host, monkeypatch):
    win, view = native_host.window, _View()
    win._activate_tool("paint")
    face = _quad(win)
    face.attrs["texture"] = {"path": "ladrillo.png", "sw": 2.5, "sh": .75, "rot": 30.}
    _sample(win, face, monkeypatch)
    rows = shown(view, win)
    assert [float(rows[k]) for k in ["Tile width (m)", "Tile height (m)", "Rotation (°)"]] == [2.5, .75, 30.]
    wgpu_library.edit_field(win, view, "brush_sw:", "3.5")
    assert PaintTool.current_texture == {**face.attrs["texture"], "sw": 3.5}
    assert face.attrs["texture"]["sw"] == 2.5
