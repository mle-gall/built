"""Orbit turns around what you are looking at, not the world origin (#164).

«Instead of orbiting around the center of the current view it seems like
it's orbiting around origin, which for a larger model is far away.» Each
orbit gesture now picks its pivot once, at the press: the model point
under the cursor, else the middle of what is on screen, else the target —
and the whole camera turns rigidly about it, so that point stays put."""
from __future__ import annotations

import math

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from core.input_types import Input
from ingetrazo_render import MouseEvent
from ingetrazo_core import Vec3 as V


W, H = 800, 600


def _box(scene, x0, y0, z0, x1, y1, z1):
    p = [V(x0, y0, z0), V(x1, y0, z0), V(x1, y1, z0), V(x0, y1, z0),
         V(x0, y0, z1), V(x1, y0, z1), V(x1, y1, z1), V(x0, y1, z1)]
    for idx in ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5),
                (2, 3, 7, 6), (3, 0, 4, 7)):
        scene.mesh.add_face([p[i] for i in idx])
    scene.version += 1


def _ev(kind, x, y, button, buttons):
    return MouseEvent(kind, x, y, button, buttons,
                       Input.NoModifier)


def _drag(vp, x0, y0, x1, y1, button=Input.MiddleButton):
    vp.mousePressEvent(_ev("press", x0, y0, button, button))
    steps = 6
    for i in range(1, steps + 1):
        x = x0 + (x1 - x0) * i / steps
        y = y0 + (y1 - y0) * i / steps
        vp.mouseMoveEvent(_ev("move", x, y, Input.NoButton, button))
    vp.mouseReleaseEvent(_ev("release", x1, y1, button,
                             Input.NoButton))


def _screen(vp, p: V):
    cam = vp.camera
    ndc = (cam.projection_matrix() * cam.view_matrix()).map(p)
    return ((ndc.x() + 1.0) * 0.5 * vp.width(), (1.0 - ndc.y()) * 0.5 * vp.height())


def _window_far_box(native_host):
    """A 10 m box at (500, 500), the camera aimed a little beside it: the
    target is NOT on the box, as after a pan or a zoom elsewhere."""

    win = native_host.window
    vp = win.viewport
    native_host.resize(W, H, 1.0)
    vp.scene.mesh = type(vp.scene.mesh)()
    vp.scene.groups = []
    _box(vp.scene, 495, 495, 0, 505, 505, 10)
    cam = vp.camera
    cam.aspect = W / H
    cam.perspective = True
    cam.target = V(485, 490, 0)          # beside the box
    cam.distance = 60.0
    cam.yaw, cam.pitch = math.radians(-60), math.radians(25)
    return win, vp


def test_the_orbit_tool_drag_turns_about_the_same_pivot(native_host):
    win, vp = _window_far_box(native_host)
    vp.set_nav_mode("orbit")
    box_corner = V(505, 495, 10)
    sx, sy = _screen(vp, box_corner)
    pivot = vp._orbit_pivot_at(sx, sy)
    _drag(vp, sx, sy, sx - 90, sy + 40, Input.LeftButton)
    ax, ay = _screen(vp, pivot)
    assert abs(ax - sx) < 1.5 and abs(ay - sy) < 1.5


def test_over_empty_sky_it_turns_about_the_middle_of_the_model_not_the_origin(native_host):
    win, vp = _window_far_box(native_host)
    vp.camera.target = V(500, 500, 5)        # the box in the middle
    pivot = vp._orbit_pivot_at(5, 5)          # a corner: sky
    assert (pivot - V(500, 500, 5)).length() < 12.0, pivot
    assert pivot.length() > 600, "it pivoted on the world origin"


def test_an_empty_scene_turns_about_the_target(native_host):
    vp = native_host.window.viewport
    native_host.resize(W, H, 1.0)
    vp.scene.mesh = type(vp.scene.mesh)()
    vp.scene.groups = []
    vp.scene.version += 1
    vp.camera.target = V(3, 4, 5)
    assert vp._orbit_pivot_at(W / 2, H / 2) == V(3, 4, 5)


def test_orbit_about_keeps_distance_level_horizon_and_the_pivot():
    from core.camera import OrbitCamera
    cam = OrbitCamera()
    cam.target = V(10, 0, 0)
    cam.distance = 30.0
    pivot = V(12, 3, 1)
    rel_before = (pivot - cam.eye()).length()
    cam.orbit_about(pivot, 50, -30, 600)
    assert abs(cam.distance - 30.0) < 1e-9
    # rigid about the pivot: the eye keeps its distance to it
    assert abs((pivot - cam.eye()).length() - rel_before) < 1e-4   # QVector3D is float32
    # the pitch clamp still holds at the poles
    for _ in range(40):
        cam.orbit_about(pivot, 0, 400, 600)
    assert cam.pitch <= math.radians(89.0) + 1e-9
