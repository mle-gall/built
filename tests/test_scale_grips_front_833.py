# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Scale grips at the front of the box are drawn over — and picked
before — the ones behind them (#833): the back grips used to be painted on
top of the front ones, in whatever order the box listed them."""
from __future__ import annotations


from built_render import AnnotationCanvas
from built_core import Vec3 as V


from tools.scale import ScaleTool  # noqa: E402


def _viewport():
    from tests.controller_surface import ControllerSurface as Viewport
    vp = Viewport(None)
    vp.resize(400, 400)
    cam = vp.camera
    cam.target, cam.distance = V(0, 0, 1), 20.0
    cam.set_view("top")
    return vp


class _Box:
    """Two grips on one vertical line: under a top view they share a pixel."""

    def __init__(self, order):
        self.order = order

    def scale_box_state(self):
        grips = {"top": (V(0, 0, 2), "hover"), "bottom": (V(0, 0, 0), "idle")}
        return {"segments": [], "grips": [grips[k] for k in self.order]}


def _centre_colour(vp, order):
    vp.active_tool = _Box(order)
    x, y = vp._world_to_pixel(V(0, 0, 2))
    canvas = AnnotationCanvas(400, 400, 400)
    vp._draw_scale_box(canvas)
    rgba = canvas.composite(b"\xff" * (400 * 400 * 4))
    pixel = (int(y) * 400 + int(x)) * 4
    return rgba[pixel:pixel + 3].hex()


def test_the_front_grip_is_drawn_on_top_whatever_the_order():
    vp = _viewport()
    red = "dc2828"
    assert _centre_colour(vp, ["top", "bottom"]) == red
    assert _centre_colour(vp, ["bottom", "top"]) == red


def test_the_front_grip_is_the_one_picked():
    vp = _viewport()
    scene = vp.scene
    f1 = scene.mesh.add_face([V(0, 0, 0), V(4, 0, 0), V(4, 3, 0), V(0, 3, 0)])
    f2 = scene.mesh.add_face([V(0, 0, 2), V(4, 0, 2), V(4, 3, 2), V(0, 3, 2)])
    scene.selection.update([f1, f2])
    tool = ScaleTool()
    tool.on_activate(vp)
    px = vp._world_to_pixel(V(0, 0, 2))
    g = tool._grip_under(vp, px[0], px[1])
    assert g is not None
    assert abs(tool._grip_pos(g).z() - 2.0) < 1e-6
