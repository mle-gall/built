# SPDX-License-Identifier: GPL-3.0-or-later
"""The Eraser takes only edges the user can see (#663, @SPMech).

A stroke across a face, aimed at an edge drawn on it, also marked the edge
on the far side of the box that happened to lie under the cursor. Select
stopped picking hidden edges in #71; the Eraser now asks the same question.
"""
from __future__ import annotations

import math

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_core import Vec3 as V


def _box_seen_from_the_front(native_host):
    """A 2 m box, a line splitting its BACK face at x = 1, seen from the
    front: the line lies right behind the front face."""
    from tests import test_pushpull_ux as T

    win = native_host.window
    win._confirm_discard = lambda *a, **k: True   # never a modal on close
    native_host.resize(1200, 800, 1.0)
    vp = win.viewport
    T._cube(vp.scene, vp.history, size=2.0, height=2.0)
    back = vp.scene.mesh.add_edge(V(1, 2, 0), V(1, 2, 2))
    vp.scene.version += 1
    cam = vp.camera
    cam.target = V(1, 1, 1)
    cam.distance = 8.0
    cam.pitch = math.radians(10.0)
    cam.yaw = math.radians(-90.0)
    vp.update()
    return win, vp, back


def test_a_stroke_over_the_front_face_leaves_the_edge_behind(native_host):
    from tools.eraser import EraserTool
    win, vp, back = _box_seen_from_the_front(native_host)
    px, py = vp._world_to_pixel(V(1, 2, 1))
    assert vp.pick_edge(px, py) == back      # it IS under the cursor
    tool = EraserTool()
    tool._mark(vp, px, py)
    assert back not in tool.marked
