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


@pytest.mark.parametrize("direction", ["front", "back", "left", "right", "top", "bottom", "iso", "face"])
def test_alignment_keeps_screen_centre_and_scale(native_host, direction):
    from built_core import Vec3
    from types import SimpleNamespace

    win = native_host.window
    vp = win._active_view()
    cam = vp.camera
    cam.target = Vec3(18000, 65000, 3)
    cam.distance = 30
    cam.aspect = 1.6
    focus = cam.eye() + cam.forward() * 7
    display = SimpleNamespace(camera=cam, width=lambda: 1600, height=lambda: 1000,
                              renderer=SimpleNamespace(pick=lambda *a: 0),
                              _model_point_at=lambda x, y: focus if (x, y) == (800, 500) else None)
    vp.keys_stand_in = display

    def projected_unit():
        right = Vec3(-math.sin(cam.yaw), math.cos(cam.yaw), 0)
        m = cam.projection_matrix() * cam.view_matrix()
        return m.map(focus + right).x() - m.map(focus).x()

    scale = projected_unit()
    if direction == "face":
        win._on_align_view_to_face(SimpleNamespace(normal=lambda: Vec3(0, -1, 0)))
    else:
        win._on_standard_view(direction)
    assert cam.target.toTuple() == pytest.approx(focus.toTuple())
    projected = (cam.projection_matrix() * cam.view_matrix()).map(focus)
    assert (projected.x(), projected.y()) == pytest.approx((0, 0), abs=1e-9)
    assert projected_unit() == pytest.approx(scale)
    cam.orbit(10, 5, 500)
    assert projected_unit() == pytest.approx(scale)
    assert cam.projection_mode() == "perspective"


@pytest.mark.parametrize("fallback", ["selection", "visible", "target"])
def test_alignment_with_empty_screen_centre(native_host, fallback):
    from built_core import Vec3
    from types import SimpleNamespace

    vp = native_host.window.viewport
    cam = vp.camera
    cam.target = Vec3(37, 8, 2)
    expected = cam.target
    point = Vec3(3, 8, 9)
    if fallback == "selection":
        face = vp.scene.mesh.add_face([(1, 2, 0), (3, 2, 0), (2, 4, 0)])
        vp.scene.selection.add(face)
        expected = Vec3(2, 3, 0)
    elif fallback == "visible":
        expected = point
    renderer = SimpleNamespace(pick=lambda *a: 0,
                               visible_focus_pixel=lambda: None if fallback == "target" else (60, 80))
    vp.keys_stand_in = SimpleNamespace(camera=cam, renderer=renderer, devicePixelRatioF=lambda: 2,
                                       width=lambda: 900, height=lambda: 600,
                                       _model_point_at=lambda x, y: point if (x, y) == (30.25, 40.25) else None)
    native_host.window._on_standard_view("right")
    assert cam.target.toTuple() == pytest.approx(expected.toTuple())


@pytest.mark.parametrize("auto", [True, False])
@pytest.mark.parametrize("path", ["middle", "orbit", "gizmo", "look_around", "walk", "rotate"])
def test_orbit_paths_with_a_rebased_display(native_host, auto, path):
    from built_core.settings import settings
    win = native_host.window
    view = native_host.view
    source = win._active_view().camera
    win.viewport.scene.mesh.add_face([(18000, 65000, 0), (18002, 65000, 0), (18000, 65002, 0)])
    view.set_scene(win.viewport.scene)
    win._on_zoom_extents()
    view._sync_scene_camera()
    assert source is not view.camera
    settings().setValue("nav/auto_perspective", "1" if auto else "0")
    source.set_projection("perspective")
    native_host.menu("view:front")
    assert source.projection_mode() == ("parallel" if auto else "perspective")
    view._sync_scene_camera()
    if path == "rotate":
        native_host.gesture("rotate", 10., 300., 200.)
    elif path == "walk":
        win._activate_tool("walk")
        tool = win.viewport.active_tool
        tool.step(win.viewport, tool.FULL_PX, 0., 0.5)
    elif path == "gizmo":
        view_commands.run(win, "gizmo_orbit:10,4")
    else:
        button = 4 if path == "middle" else 1
        if path == "orbit":
            win._activate_nav("orbit")
        elif path == "look_around":
            win._activate_tool(path)
        native_host.pointer("press", 300, 200, button=button, held=button)
        native_host.pointer("move", 325, 210, held=button)
        native_host.pointer("release", 325, 210, button=button)
    view._sync_scene_camera()
    assert source.yaw != pytest.approx(-math.pi / 2)
    assert source.projection_mode() == view.camera.projection_mode() == "perspective"


def test_native_view_menu_keeps_zoom_extents_separate(native_host):
    win = native_host.window
    vp = win._active_view()
    vp.scene.mesh.add_face([(0, 0, 0), (60, 0, 0), (0, 60, 0)])
    cam = vp.camera
    cam.target = (2, 3, 4)
    cam.distance = 0.5
    native_host.menu("view:front")
    assert cam.distance == pytest.approx(0.5)
    assert cam.target.toTuple() == pytest.approx((2, 3, 4))
    native_host.menu("view:zoom_extents")
    assert cam.distance > 10


def test_face_alignment_respects_disabled_auto_perspective(native_host):
    from built_core import Vec3
    from built_core.settings import settings
    from types import SimpleNamespace

    win = native_host.window
    cam = win._active_view().camera
    settings().setValue("nav/auto_perspective", "0")
    cam.set_projection("two_point")
    win._on_align_view_to_face(SimpleNamespace(normal=lambda: Vec3(0, -1, 0)))
    assert cam.projection_mode() == "two_point"
    cam.orbit(12, 0, 500)
    assert cam.projection_mode() == "two_point"
