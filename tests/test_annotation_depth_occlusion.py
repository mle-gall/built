# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The overlay's annotations (dimensions, leader texts) learn what is hidden
from the depth the frame just rendered, not from ray casts: a .skp (2018)
house with 25 dimensions spent 2.8 s of EVERY frame casting 1350 rays at
284 000 triangles (26-09-2026); an orbit frame went from 1722 ms to 50 ms.

Requires a native wgpu surface for pixel readback."""
from __future__ import annotations

from tests.shell_isolation import native_gpu_host, restore_shell_patches  # noqa: F401
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from tests.controller_surface import live_image
import pytest
from ingetrazo_core import Vec3 as V
def _box(scene, x0, y0, z0, x1, y1, z1):
    p = [V(x0, y0, z0), V(x1, y0, z0), V(x1, y1, z0), V(x0, y1, z0),
         V(x0, y0, z1), V(x1, y0, z1), V(x1, y1, z1), V(x0, y1, z1)]
    for idx in ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5),
                (2, 3, 7, 6), (3, 0, 4, 7)):
        scene.mesh.add_face([p[i] for i in idx])


def test_a_point_behind_a_wall_is_hidden_and_one_in_front_is_not(native_gpu_host):
    from core.dimension import Dimension
    win = native_gpu_host.window
    vp = win.viewport
    sc = vp.scene
    _box(sc, -2, -0.1, 0, 2, 0.1, 3)                 # a wall across Y=0
    sc.dimensions.append(Dimension(V(-1, 2, 1), V(1, 2, 1), V(0, 0, 0.5)))
    sc.version += 1
    vp.camera.set_view("front")                      # looking toward +Y
    vp.camera.perspective = False
    vp.camera.fit_to(V(-3, -1, 0), V(3, 3, 3))
    live_image(vp)       # a real frame, painted the way Qt does
    behind = V(0, 2, 1)                              # the wall hides it
    front = V(0, -2, 1)                              # nothing in front
    on_face = V(0, -0.1, 1)                          # ON the wall's face
    assert vp._occluded_on_screen(behind)
    assert not vp._occluded_on_screen(front)
    assert not vp._occluded_on_screen(on_face)
    # the rays agree on all three
    assert vp._is_occluded(behind)
    assert not vp._is_occluded(front)
    assert not vp._is_occluded(on_face)
