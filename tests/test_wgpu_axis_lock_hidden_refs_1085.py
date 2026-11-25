# SPDX-License-Identifier: GPL-3.0-or-later
"""Upstream #1085 on the Metal view: with Z locked from a box's top corner,
moving the cursor straight up takes the point up, not down to the height
of a hidden back edge or of its midpoint. In X-ray nothing hides anything,
so the back edge still lends its height there.

Drives an unshown ``WgpuView`` with the real Line tool (the fixture of
``test_inference_and_face_cutting``): the rules are ``built_core.snap``'s, the
candidates and the occlusion test are the Metal view's own."""
import math

import pytest
from built_core import Vec3 as V
from tests.headless_gpu import mouse

from tests.test_inference_and_face_cutting import click, refresh, view  # noqa: F401


def _box(mesh, x0, y0, z0, dx, dy, dz):
    x1, y1, z1 = x0 + dx, y0 + dy, z0 + dz
    p = [V(x0, y0, z0), V(x1, y0, z0), V(x1, y1, z0), V(x0, y1, z0),
         V(x0, y0, z1), V(x1, y0, z1), V(x1, y1, z1), V(x0, y1, z1)]
    for a, b, c, d in ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4),
                       (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)):
        mesh.add_face([p[a], p[b], p[c], p[d]])


def _iso(view):
    """Upstream's frame: 1000x700, Iso-like, the 2 m box at the origin (at
    800x600 the far top corner sits 120 px above the near one, a visible
    reference that rightly gives its own height)."""
    _box(view.scene.mesh, 0, 0, 0, 2, 2, 2)
    host = getattr(view, "_host", None)
    if callable(getattr(host, "resize", None)):          # the shell's host
        host.resize(1000, 700, 1.)
    else:                                                 # a headless view
        view.resize(1000, 700)
        view.source_viewport.resize(1000, 700)
    cam = view.camera
    cam.aspect = 1000 / 700
    cam.target = V(1, 1, 1)
    cam.distance = 9
    cam.yaw = math.radians(45)
    cam.pitch = math.radians(35)
    refresh(view)


def _move_px(view, x, y):
    mouse(view, "move", x, y)
    return view.last_snap


def _z_locked_from(view, corner):
    click(view, corner)
    assert view.tools.start_point is not None
    assert (view.tools.start_point - corner).length() < 1e-6
    view.source_viewport.inference.axis_lock = "z"
    return view._world_to_pixel(corner)


def test_metal_z_lock_from_the_top_corner_goes_up(view):
    _iso(view)
    corner = V(2, 2, 2)
    from built_core import occluded as _occluded
    # The trap is real in this view: the far edge is hidden.
    assert _occluded(view.inference_index, view, V(0, 0, 1.0))
    a = _z_locked_from(view, corner)
    heights = []
    for up in (40, 80, 120, 200):
        snap = _move_px(view, a[0], a[1] - up)
        assert snap is not None
        heights.append(snap.point.z())
    assert all(h > corner.z() for h in heights), heights
    assert heights == sorted(heights), heights


def test_metal_xray_still_takes_the_back_edges_height(view):
    _iso(view)
    view.scene.display_style.face_mode = "xray"
    assert view.source_viewport._effective_style().face_mode == "xray"
    a = _z_locked_from(view, V(2, 2, 2))
    back_mid = view._world_to_pixel(V(0, 0, 1))      # seen through in X-ray
    snap = _move_px(view, *back_mid)
    assert snap is not None
    assert snap.point.z() == pytest.approx(1.0, abs=1e-6)
    assert a is not None
