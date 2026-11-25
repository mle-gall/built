# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Opening a document asks the view for a frame once it is in. Painting is
off while it loads, and turning it back on asks for none: the view stayed
black until the first click."""
from __future__ import annotations

from built_core import Vec3 as V

from core.group import Group
from core.mesh import Mesh
from core.scene import Scene
from formats import igz
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_the_view_is_asked_to_paint_after_opening(native_host, tmp_path, monkeypatch):
    scene = Scene()
    m = Mesh()
    m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    scene.groups.append(Group(m, name="Caja"))
    path = tmp_path / "caja.igz"
    igz.save_scene(scene, path)
    win = native_host.window
    frames = []
    original = native_host.request_frame

    def request_frame():
        frames.append([g.name for g in win.viewport.scene.groups])
        original()

    monkeypatch.setattr(native_host, "request_frame", request_frame)
    assert win.open_path(path)
    assert ["Caja"] in frames
