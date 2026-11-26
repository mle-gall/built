"""Auto perspective and the axis gizmo's commands (UX pass 2, M03).

Aligning to a standard view shows it in parallel; the first orbit away
returns to the projection the user had. Settings > Navigation > Views >
Auto perspective turns it off.
"""
import math

import pytest
from built_core.camera import OrbitCamera

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import view_commands


def test_the_camera_returns_to_its_projection_on_the_first_orbit():
    cam = OrbitCamera()
    cam.set_projection("two_point")
    cam.align_to("front")
    assert cam.projection_mode() == "parallel" and cam.auto_projection_before() == "two_point"
    cam.align_to("top")
    cam.zoom(1.0)
    cam.pan(10, 0, 500)
    assert cam.projection_mode() == "parallel"
    cam.orbit(10, 0, 500)
    assert cam.projection_mode() == "two_point" and cam.auto_projection_before() is None
    cam.orbit(10, 0, 500)
    assert cam.projection_mode() == "two_point"


def test_isometric_keeps_the_users_projection_and_a_choice_by_hand_sticks():
    cam = OrbitCamera()
    cam.align_to("right")
    cam.align_to("iso")
    assert cam.projection_mode() == "perspective"
    assert cam.yaw == pytest.approx(math.radians(-45)) or cam.yaw == pytest.approx(math.radians(45))
    cam.align_to("left")
    cam.set_projection("parallel")
    cam.orbit(10, 0, 500)
    assert cam.projection_mode() == "parallel"


def test_setting_off_leaves_the_projection_alone():
    cam = OrbitCamera()
    cam.align_to("front", False)
    assert cam.projection_mode() == "perspective"
    assert (cam.yaw, cam.pitch) == pytest.approx((math.radians(-90), 0))


def test_a_face_alignment_follows_the_same_rule():
    cam = OrbitCamera()
    yaw0, pitch0 = cam.yaw, cam.pitch
    cam.yaw, cam.pitch = 0.3, 0.0
    cam.align_current(yaw0, pitch0)
    assert cam.projection_mode() == "parallel"
    cam.orbit(0, 12, 500)
    assert cam.projection_mode() == "perspective"


def test_view_commands_and_the_gizmo_drag_in_the_window(native_host):
    from built_core.settings import settings as preferences
    win = native_host.window
    cam = win._active_view().camera
    cam.set_projection("perspective")
    assert view_commands.run(win, "view_front")
    assert cam.projection_mode() == "parallel"
    assert (cam.yaw, cam.pitch) == pytest.approx((math.radians(-90), 0))
    # The gizmo's drag orbits the active view: perspective again.
    assert view_commands.run(win, "gizmo_orbit:14,-3")
    assert cam.projection_mode() == "perspective"
    assert cam.yaw != pytest.approx(math.radians(-90))
    settings = preferences()
    settings.setValue("nav/auto_perspective", "0")
    try:
        assert view_commands.run(win, "view_top")
        assert cam.projection_mode() == "perspective"
    finally:
        settings.remove("nav/auto_perspective")
