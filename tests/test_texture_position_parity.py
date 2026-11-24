# SPDX-License-Identifier: GPL-3.0-or-later
"""Replay the placement and transient pin state recorded from the original tool."""
import json
from types import SimpleNamespace

import pytest
from ingetrazo_core import Vec3
from PySide6.QtCore import QPointF, Qt

from core.geometry import Face
from tests.tool_parity import FIXTURES, _pt, _r
from tools.base import ToolContext
from tools.texture_position import TexturePositionTool


SCENARIOS = {
    "lift_hold_place_rotate": [
        ("click", .5, 0), ("release",), ("hover", .04, .02),
        ("click", .04, .02), ("click", 1, 0), ("hover", .71, .59),
        ("hover", .71, .59, True), ("hover", .03, 1.02), ("release",)],
    "shear_flip_rotate_reset_undo": [
        ("click", .5, .5), ("hover", .85, .79), ("release",),
        ("flip", True), ("rotate", 90), ("reset",), ("undo_step",)],
    "body_drag_restore_exit": [
        ("click", 1.4, 1.5), ("hover", 1.43, 1.5), ("hover", 1.6, 1.7),
        ("release",), ("on_cancel",), ("on_cancel",)],
}


def replay(steps):
    vp = SimpleNamespace(_world_to_pixel=lambda p: (100*p.x(), 100*p.y()),
                         _pixel_to_ray=lambda x, y: (Vec3(x/100, y/100, 1), Vec3(0, 0, -1)),
                         _ray_plane=lambda o, *args: Vec3(o.x(), o.y(), 0),
                         flash_status=lambda *args: None, update=lambda: None)
    face = Face([Vec3(0, 0, 0), Vec3(2, 0, 0), Vec3(2, 2, 0), Vec3(0, 2, 0)],
                [], {"texture": {"path": "brick.png", "sw": .5, "sh": .5}})
    tool = TexturePositionTool()
    assert tool.begin(vp, face, Vec3(.7, .3, 0))
    states = []
    for op, *args in steps:
        if op in ("hover", "click"):
            x, y, *ctrl = args
            ctx = ToolContext(vp, Vec3(x, y, 0), QPointF(100*x, 100*y),
                              Qt.ControlModifier if ctrl else Qt.NoModifier,
                              None)
            getattr(tool, "on_" + op)(ctx)
        elif op in ("release", "on_cancel"):
            getattr(tool, "on_release" if op == "release" else op)(vp)
        else:
            getattr(tool, op)(*args)
        states.append(dict(pins=[_pt(p) for p in tool.pins()],
                           uvw=[_r(v) for v in tool.map.uvw()] if tool.active else None,
                           lifted=tool._lifted, sweep=tool._sweep_deg,
                           held=tool._held[1] if tool._held else None,
                           changed=tool.changed()))
    return states


@pytest.mark.parametrize("name", SCENARIOS)
def test_position_matches_the_python_gesture_sequence(name):
    expected = json.loads((FIXTURES / "texture_position.json").read_text())
    assert replay(SCENARIOS[name]) == expected[name]
