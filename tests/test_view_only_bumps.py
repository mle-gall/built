# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A click changes the view, not the model (#158, «sin lag»).

On the plant of #158 each selection click rebuilt the pick index, the
silhouettes and every edge buffer, and rescanned the side panels — over a
second a click. A view-only bump (``Scene.bump_view``) now leaves what the
model IS alone and refreshes only what shows the selection."""
from __future__ import annotations


from tests.shell_isolation import restore_shell_patches  # noqa: F401
from built_core import Matrix4 as QMatrix4x4, Vec3 as V


def _window(native_host):
    from core.group import Group
    from core.mesh import Mesh

    win = native_host.window
    native_host.resize(800, 600, 1.0)
    m = Mesh()
    m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    vp = win.viewport
    for i in range(3):
        g = Group(m, name=f"G{i}")
        x = QMatrix4x4()
        x.translate(2.0 * i, 0.0, 0.0)
        g.xform = x
        vp.scene.groups.append(g)
    vp.scene.mesh.add_face([V(0, 3, 0), V(1, 3, 0), V(1, 4, 0)])
    vp.scene.version += 1
    return win


def test_a_selection_keeps_the_pick_index(native_host):
    win = _window(native_host)
    vp = win.viewport
    idx = vp._pick_index()
    cv = vp.scene.content_version
    vp.scene.select([vp.scene.groups[1]])
    assert vp.scene.content_version == cv
    assert vp._pick_index() is idx
    vp.scene.version += 1                       # an edit: a new one
    assert vp._pick_index() is not idx
