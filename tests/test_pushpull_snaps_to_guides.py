# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Push/Pull stops level with a Tape Measure guide (issue #165).

The usual way: you mark a height with the Tape -- a guide point above a corner,
or a guide line -- and push the face until it engages the guide. Ours never
engaged: the push's distance inference scanned mesh vertices, edges and faces,
and guides live in ``Scene.guides``, outside the mesh.
"""
from __future__ import annotations

import math

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from core.input_types import ScreenPoint, Input
from ingetrazo_render import MouseEvent
from ingetrazo_core import Vec3 as QVector3D

from core.guide import Guide


def _window(native_host):
    win = native_host.window
    native_host.resize(1600, 900, 1.0)
    vp = win.viewport
    cam = vp.camera
    cam.target = QVector3D(2, 2, 1.5)
    cam.distance = 16.0
    cam.pitch = math.radians(25.0)
    cam.yaw = math.radians(-50.0)
    face = vp.scene.mesh.add_face([QVector3D(0, 0, 0), QVector3D(4, 0, 0),
                                   QVector3D(4, 4, 0), QVector3D(0, 4, 0)])
    vp.scene.version += 1
    vp.update()
    return win, vp, face


def _hover(vp, px, py):
    vp._last_mouse_pos = ScreenPoint(px, py)
    vp._process_hover(ScreenPoint(px, py), Input.NoModifier)


def _click(vp, px, py):
    vp._last_mouse_pos = ScreenPoint(px, py)
    vp._dispatch_tool_click(MouseEvent(
        "press", px, py, Input.LeftButton, Input.LeftButton, Input.NoModifier))


def _start_push(win, vp):
    win._activate_tool("pushpull")
    cx, cy = vp._world_to_pixel(QVector3D(2, 2, 0))
    _hover(vp, cx, cy)
    tool = vp.active_tool
    _click(vp, cx, cy)
    assert tool.dragging
    return tool


def _tape_guide_point(win, vp, height):
    """The Tape's own way: click a corner, aim up the blue axis, type."""
    win._activate_tool("tape")
    tape = vp.active_tool
    tape._mode = "point"
    x0, y0 = vp._world_to_pixel(QVector3D(4, 4, 0))
    _hover(vp, x0, y0)
    _click(vp, x0, y0)
    x1, y1 = vp._world_to_pixel(QVector3D(4, 4, 2.0))
    _hover(vp, x1, y1)
    assert tape.on_value(vp, height)
    points = [g for g in vp.scene.guides if g.direction is None]
    assert len(points) == 1, vp.scene.guides
    return points[0]


def test_push_stops_level_with_a_tape_guide_point(native_host):
    win, vp, face = _window(native_host)
    guide = _tape_guide_point(win, vp, 2.5)
    tool = _start_push(win, vp)
    gx, gy = vp._world_to_pixel(guide.point)
    _hover(vp, gx + 2, gy + 1)                   # a hair off the guide
    assert tool._inference_kind == "guide_point"
    assert abs(tool.extrusion - guide.point.z()) < 1e-9
    marker = tool.inference_marker()
    assert marker is not None and marker[1] == "guide_point"
    _click(vp, gx + 2, gy + 1)                   # commit
    tops = [f for f in vp.scene.mesh.faces
            if all(abs(v.z() - guide.point.z()) < 1e-9 for v in f.vertices)]
    assert tops, "no cap at the guide's height"


def test_push_stops_level_with_a_guide_line(native_host):
    win, vp, face = _window(native_host)
    # the kind of line the Tape pulls off an edge: along X, 3 m up
    line = Guide(QVector3D(0, 6, 3.0), QVector3D(1, 0, 0))
    vp.scene.guides.append(line)
    tool = _start_push(win, vp)
    lx, ly = vp._world_to_pixel(QVector3D(2, 6, 3.0))
    _hover(vp, lx + 1, ly + 2)
    assert tool._inference_kind == "guide_line"
    assert abs(tool.extrusion - 3.0) < 1e-9


def test_without_the_guide_the_push_follows_the_cursor(native_host):
    win, vp, face = _window(native_host)
    tool = _start_push(win, vp)
    target = QVector3D(4, 4, 2.5)
    gx, gy = vp._world_to_pixel(target)
    _hover(vp, gx + 2, gy + 1)
    assert tool._inference_kind != "guide_point"
    assert abs(tool.extrusion - 2.5) > 1e-6       # nothing to lock on
