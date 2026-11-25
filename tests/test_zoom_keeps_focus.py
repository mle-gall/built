# SPDX-License-Identifier: GPL-3.0-or-later
"""A zoom about the cursor keeps the point under the cursor under the cursor.

Owner, 2026-11-25, ride file, trackpad: "zooming out drifts toward the left
of the zoom origin point". Close to a detail of a large model every zoom-out
step is shorter than the retreat floor (1 % of the model's diagonal a step).
The floor used to push the eye straight back along the view axis after the
zoom about the focus, so a focus beside the centre of the window slid toward
the centre at every step, and zooming back in did not undo it."""
from __future__ import annotations

import math

import pytest

from built_core import Vec3 as V

from core.camera import OrbitCamera

WIDTH, HEIGHT = 1440.0, 900.0


def close_up(perspective=True):
    cam = OrbitCamera()
    cam.set_aspect(WIDTH, HEIGHT)
    cam.perspective = perspective
    cam.yaw, cam.pitch = math.radians(35.0), math.radians(25.0)
    cam.target = V(40.0, 12.0, 6.0)
    cam.distance = 3.0
    return cam


def pixel(cam, world):
    ndc = (cam.projection_matrix() * cam.view_matrix()).map(world)
    return ((ndc.x() + 1.0) * WIDTH / 2.0, (1.0 - ndc.y()) * HEIGHT / 2.0)


def beside_the_centre(cam):
    """A point of the scene a third of the window right of and above its centre."""
    forward = cam.forward()
    right = V.crossProduct(forward, cam.up_vector()).normalized()
    up = V.crossProduct(right, forward).normalized()
    return cam.target + right * 0.9 + up * 0.5 + forward * 0.4


def drift(cam, focus, steps, retreat):
    """The farthest the focus gets from its first pixel over the given zoom steps."""
    x0, y0 = pixel(cam, focus)
    worst = 0.0
    for step in steps:
        cam.zoom_to(step, focus, min_step=retreat * abs(step))
        x, y = pixel(cam, focus)
        worst = max(worst, math.hypot(x - x0, y - y0))
    return worst


# A 300 m model: the floor is 3 m a step, the plain 10 % step from 3 m is 0.33 m.
RETREAT = 3.0
OUT_THEN_IN = [-1.0] * 6 + [1.0] * 6
SLIDE = [-0.05] * 120 + [0.05] * 120      # a trackpad slide out and back


@pytest.mark.parametrize("steps", [OUT_THEN_IN, SLIDE], ids=["notches", "slide"])
@pytest.mark.parametrize("perspective", [True, False], ids=["perspective", "parallel"])
def test_zooming_out_then_in_keeps_the_focus_under_the_cursor(steps, perspective):
    cam = close_up(perspective)
    focus = beside_the_centre(cam)
    x, y = pixel(cam, focus)
    assert abs(x - WIDTH / 2) > 100 and abs(y - HEIGHT / 2) > 50      # really off centre
    assert drift(cam, focus, steps, RETREAT) < 0.01                    # pixels


def test_the_retreat_floor_still_moves_the_eye_by_its_step():
    cam = close_up()
    focus = beside_the_centre(cam)
    eye = cam.eye()
    cam.zoom_to(-1.0, focus, min_step=RETREAT)
    assert (cam.eye() - eye).length() == pytest.approx(RETREAT, abs=1e-6)


def test_without_a_floor_the_zoom_is_the_plain_ten_percent_step():
    cam = close_up()
    focus = beside_the_centre(cam)
    reach = (cam.eye() - focus).length()
    cam.zoom_to(-1.0, focus)
    assert cam.distance == pytest.approx(3.0 / 0.9)
    assert (cam.eye() - focus).length() == pytest.approx(reach / 0.9)


def test_the_floor_stops_at_the_far_limit():
    from core.camera import MAX_DISTANCE
    cam = close_up()
    focus = beside_the_centre(cam)
    cam.zoom_to(-1.0, focus, min_step=MAX_DISTANCE * 10)
    assert cam.distance <= MAX_DISTANCE
    assert cam.is_finite()
