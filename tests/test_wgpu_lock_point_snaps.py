# SPDX-License-Identifier: GPL-3.0-or-later
"""Point snaps under a direction lock on the Metal view (upstream #1093).

Under the arrow lock or a Shift lock (here the perpendicular to the edge the
line started on), a midpoint, the world origin or a corner sitting ON the
lock line is the exact point, named as such: the lock used to offer only
edge ends there, and a midpoint or the origin on the line came back as a
'from point' foot or the cursor's own projection.

Drives an unshown ``WgpuView`` with the real Line tool (the fixture of
``test_inference_and_face_cutting``)."""
import pytest
from built_core import Vec3 as V
from tests.headless_gpu import mouse

from tests.test_inference_and_face_cutting import click, refresh, view  # noqa: F401


def _scene(view):
    mesh = view.scene.mesh
    mesh.add_edge(V(-2, 1, 0), V(-2, 3, 0))      # the edge the line starts on
    mesh.add_edge(V(1, 1, 0), V(5, 1, 0))        # collinear with the lock line
    mesh.add_edge(V(-1, 0, 0), V(-1, -1, 0))     # puts nothing on the line
    view.camera.target = V(1.5, 1, 0)
    view.camera.distance = 12
    refresh(view)


def _move_px(view, x, y):
    mouse(view, "move", x, y)
    return view.last_snap


def _lock(view, mode):
    inference = view.source_viewport.inference
    if mode == "arrow":
        inference.axis_lock = "x"
    else:                       # Shift on the perpendicular to the start edge
        inference._shift_lock = (V(1, 0, 0), (0.85, 0.3, 0.8))


@pytest.mark.parametrize("mode", ["arrow", "shift"])
@pytest.mark.parametrize("target,kind", [(V(3, 1, 0), "midpoint"),
                                         (V(5, 1, 0), "endpoint")])
def test_metal_point_on_the_lock_line_is_exact(view, mode, target, kind):
    _scene(view)
    click(view, V(-2, 1, 0))
    assert (view.tools.start_point - V(-2, 1, 0)).length() < 1e-6
    _lock(view, mode)
    p = view._world_to_pixel(target)
    snap = _move_px(view, p[0] + 2.0, p[1] + 1.5)     # a few px beside it
    assert snap.kind == kind, (snap.kind, snap.point)
    assert (snap.point - target).length() < 1e-6, snap.point
    assert snap.lock_color is not None


@pytest.mark.parametrize("mode", ["arrow", "shift"])
def test_metal_origin_on_the_lock_line_is_exact(view, mode):
    mesh = view.scene.mesh
    mesh.add_edge(V(-2, 0, 0), V(-2, 2, 0))
    view.camera.target = V(0, 0, 0)
    view.camera.distance = 10
    refresh(view)
    click(view, V(-2, 0, 0))
    assert (view.tools.start_point - V(-2, 0, 0)).length() < 1e-6
    _lock(view, mode)
    p = view._world_to_pixel(V(0, 0, 0))
    snap = _move_px(view, p[0] + 2.0, p[1] + 1.5)
    assert snap.kind == "origin", (snap.kind, snap.point)
    assert snap.point.length() < 1e-6
