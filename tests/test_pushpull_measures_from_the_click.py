# SPDX-License-Identifier: GPL-3.0-or-later
"""Push/Pull measures the drag from the clicked spot, not the centroid.

The pull was read off the line through the face's centroid. On a 12 × 7.32
slab clicked near a corner, lifting the cursor 0.3 m read -3.39 m; on a ring
the centroid lies in the hole and 1 m of cursor gave 0.13 m walls (tutorial
hunt 2, A13/B5). The cursor now rides the line through the click.
"""
from __future__ import annotations

import math

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from core.input_types import ScreenPoint, Input
from ingetrazo_render import MouseEvent
from ingetrazo_core import Vec3 as V


def _window(native_host, target, distance):
    win = native_host.window
    win._confirm_discard = lambda *a, **k: True   # never a modal on close
    native_host.resize(1600, 900, 1.0)
    cam = win.viewport.camera
    cam.target = target
    cam.distance = distance
    cam.pitch = math.radians(30.0)
    cam.yaw = math.radians(-50.0)
    return win, win.viewport


def _event(vp, px, py):
    vp._last_mouse_pos = ScreenPoint(px, py)
    return MouseEvent("press", px, py, Input.LeftButton, Input.LeftButton, Input.NoModifier)


def _pull(win, vp, at, rise):
    """Click the face at ``at``, move the cursor to ``rise`` m above it."""
    vp.scene.version += 1
    vp.update()
    win._activate_tool("pushpull")
    tool = vp.active_tool
    px, py = vp._world_to_pixel(at)
    vp._process_hover(ScreenPoint(px, py), Input.NoModifier)
    vp._dispatch_tool_click(_event(vp, px, py))
    assert tool.dragging
    qx, qy = vp._world_to_pixel(at + V(0, 0, rise))
    vp._process_hover(ScreenPoint(qx, qy), Input.NoModifier)
    return tool.extrusion


def test_a_pull_clicked_near_a_slab_corner_follows_the_cursor(native_host):
    win, vp = _window(native_host, V(6, 3.66, 0), 22.0)
    vp.scene.mesh.add_face([V(0, 0, 0), V(12, 0, 0), V(12, 7.32, 0),
                            V(0, 7.32, 0)])
    got = _pull(win, vp, V(0.4, 0.4, 0), 0.3)
    assert abs(got - 0.3) < 0.03, got


def test_a_ring_pulled_one_metre_rises_one_metre(native_host):
    win, vp = _window(native_host, V(0, 0, 0), 14.0)
    outer = [V(-2, -2, 0), V(2, -2, 0), V(2, 2, 0), V(-2, 2, 0)]
    hole = [V(-1.8, -1.8, 0), V(-1.8, 1.8, 0), V(1.8, 1.8, 0),
            V(1.8, -1.8, 0)]
    vp.scene.mesh.add_face(outer, [hole])
    got = _pull(win, vp, V(1.9, 0, 0), 1.0)
    assert abs(got - 1.0) < 0.05, got
