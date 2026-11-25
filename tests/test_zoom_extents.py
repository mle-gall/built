"""Zoom Extents frames the model as the camera looks at it NOW.

It used to frame the box's bounding sphere, which does not depend on the
view: after an orbit or a standard view a second Zoom Extents gave back the
same target and distance and seemed to do nothing, with a tower at 9 % of
the screen seen from above (Marco: «a veces no hace efecto, en determinadas
posiciones»), and a tall window cut the model at the sides."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
import math

import pytest
from built_core import Vec3 as V, Vec4 as QVector4D

from core.camera import MAX_DISTANCE, OrbitCamera


def _screen(cam, lo, hi):
    """(half-span of the box on screen, every corner inside the window)."""
    mvp = cam.projection_matrix() * cam.view_matrix()
    xs, ys = [], []
    for x in (lo.x(), hi.x()):
        for y in (lo.y(), hi.y()):
            for z in (lo.z(), hi.z()):
                p = mvp.map(QVector4D(x, y, z, 1.0))
                xs.append(p.x() / p.w())
                ys.append(p.y() / p.w())
    span = max(max(xs) - min(xs), max(ys) - min(ys)) / 2.0
    inside = all(abs(v) <= 1.0 + 1e-4 for v in xs + ys)
    return span, inside


BOXES = {
    "tower": (V(0, 0, 0), V(5, 5, 40)),
    "wall": (V(0, 0, 0), V(20, 0.2, 3)),
    "plaza": (V(0, 0, 0), V(30, 20, 0.2)),
    "street": (V(0, 0, 0), V(1000, 5, 0)),
}


@pytest.mark.parametrize("box", sorted(BOXES))
@pytest.mark.parametrize("view", ["top", "right", "front", "iso"])
@pytest.mark.parametrize("perspective", [True, False])
@pytest.mark.parametrize("aspect", [1.8, 0.6])
def test_the_model_fills_the_window_in_any_view(box, view, perspective, aspect):
    lo, hi = BOXES[box]
    cam = OrbitCamera()
    cam.aspect, cam.perspective = aspect, perspective
    cam.set_view(view)
    cam.fit_box(lo, hi)
    span, inside = _screen(cam, lo, hi)
    assert inside
    assert span == pytest.approx(1 / 1.1, abs=0.02)


def _extent(cam, lo, hi):
    mvp = cam.projection_matrix() * cam.view_matrix()
    pts = [mvp.map(QVector4D(x, y, z, 1.0)) for x in (lo.x(), hi.x())
           for y in (lo.y(), hi.y()) for z in (lo.z(), hi.z())]
    xs = [p.x() / p.w() for p in pts]
    ys = [p.y() / p.w() for p in pts]
    return min(xs), min(ys), max(xs), max(ys)


@pytest.mark.parametrize("box", sorted(BOXES))
@pytest.mark.parametrize("view", ["top", "front", "iso"])
@pytest.mark.parametrize("perspective", [True, False])
def test_zoom_extents_frames_into_the_free_area(box, view, perspective):
    """With the rail on the left and the dock on the right, the model is
    framed into the area they leave (council B8), not under the dock."""
    lo, hi = BOXES[box]
    free = (-0.82, -0.9, 0.45, 0.9)            # x0, y0, x1, y1 in the window
    cam = OrbitCamera()
    cam.aspect, cam.perspective = 1.6, perspective
    cam.set_view(view)
    cam.fit_box(lo, hi, free=free)
    x0, y0, x1, y1 = _extent(cam, lo, hi)
    assert x0 >= free[0] - 1e-3 and x1 <= free[2] + 1e-3
    assert y0 >= free[1] - 1e-3 and y1 <= free[3] + 1e-3
    assert (x0 + x1) / 2 == pytest.approx((free[0] + free[2]) / 2, abs=0.01)
    fill = max((x1 - x0) / (free[2] - free[0]), (y1 - y0) / (free[3] - free[1]))
    assert fill == pytest.approx(1 / 1.1, abs=0.02)


def test_a_second_zoom_extents_after_an_orbit_is_not_a_no_op():
    lo, hi = BOXES["plaza"]
    cam = OrbitCamera()
    cam.aspect = 1.8
    cam.fit_box(lo, hi)
    before = (V(cam.target), cam.distance)
    cam.yaw, cam.pitch = math.radians(-95), math.radians(-30)
    cam.fit_box(lo, hi)
    assert (cam.target, cam.distance) != before
    assert _screen(cam, lo, hi)[1]


def test_a_point_and_a_huge_model_stay_in_range():
    cam = OrbitCamera()
    cam.fit_box(V(1, 1, 1), V(1, 1, 1))
    assert cam.distance > 0 and math.isfinite(cam.distance)
    cam.fit_box(V(0, 0, 0), V(60000, 60000, 10))
    assert cam.distance <= MAX_DISTANCE


def test_zoom_extents_frames_the_scale_figure_in_a_new_document(native_window):
    """A new document holds only the scale figure; Zoom Extents used to do
    nothing there (the figure is left out of the scene bounds), where
    the classic Zoom Extents frames the figure (Marco, 23-09)."""
    win = native_window
    win._start_blank_document()
    cam = win.viewport.camera
    cam.target = V(500, 500, 0)
    cam.distance = 4000                   # far away from everything
    win._on_zoom_extents()
    assert cam.distance < 20              # framed the 1.7 m figure
    assert abs(cam.target.x() + 0.65) < 1.0 and abs(cam.target.y() + 0.6) < 1.0


def _image_window(native_host):
    from built_core.image_plane import ImagePlane
    from core.layers import Layer

    win = native_host.window
    scene = win.viewport.scene
    scene.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])
    img = ImagePlane("plan.png", V(200, 100, 0), V(20, 0, 0), V(0, 15, 0),
                     aspect=4 / 3)
    scene.image_planes.append(img)
    scene.layers.append(Layer(img.layer))
    scene.version += 1
    return win, img


def test_zoom_extents_frames_a_reference_image(native_host):
    """An imported plan is what you start tracing on, often far from the
    origin; Zoom Extents left it out (``Scene.bounds()`` covers geometry
    only), so it framed the scale figure and the plan stayed off screen —
    while Zoom Selection over the same plan did frame it."""
    win, img = _image_window(native_host)
    cam = win.viewport.camera
    win._on_zoom_extents()
    corners = img.corners()
    lo = V(min(c.x() for c in corners), min(c.y() for c in corners), 0)
    hi = V(max(c.x() for c in corners), max(c.y() for c in corners), 0)
    assert _screen(cam, lo, hi)[1]                # the whole plan in view


def test_zoom_extents_leaves_out_a_hidden_reference_image(native_host):
    win, img = _image_window(native_host)
    scene = win.viewport.scene
    scene.layer(img.layer).visible = False
    scene.version += 1
    cam = win.viewport.camera
    win._on_zoom_extents()
    assert cam.distance < 20                      # the nearby geometry alone
