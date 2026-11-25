# SPDX-License-Identifier: GPL-3.0-or-later
"""Trackpad and touch-screen navigation (#248).

A finger slide on the touch screen (the wheel events with a scroll phase
that macOS 27 synthesises on the primary pointer) orbits by default,
Shift pans, Cmd/Ctrl zooms; a two-finger slide on a real trackpad (its
own device id) zooms as it always did unless the preference says
otherwise; a pinch zooms toward the fingers through the wheel's zoom; a
slide and a pinch do not fight; the two-finger double tap frames the
model and goes back. A mouse wheel notch zooms as before.
"""
from __future__ import annotations

import math

from built_core import Vec3 as QVector3D
from tests.shell_isolation import restore_shell_patches  # noqa: F401
import pytest
from built_core import ScreenPoint as QPointF, Input as Qt
from built_core import DeviceType
from built_render import WheelEvent, NativeGestureEvent
from types import SimpleNamespace

from built_render import trackpad_nav

CENTRE = QPointF(200, 150)
SCREEN = DeviceType.TouchScreen
REAL_TRACKPAD = DeviceType.TouchPad
MOUSE = DeviceType.Mouse


@pytest.fixture
def vp():
    from tests.controller_surface import ControllerSurface as Viewport
    view = Viewport(None)
    view.resize(400, 300)
    view._touch_slide = "orbit"
    view._trackpad_slide = "zoom"
    view._invert_orbit_y = False
    view._invert_wheel = False
    return view


def slide_event(dx, dy, phase="move", mods=Qt.NoModifier,
                inverted=True, at=CENTRE, device=None):
    # Natural scrolling (the Mac default): the deltas follow the fingers.
    return WheelEvent(at.x(), at.y(), dx, dy, dx * 8, dy * 8, phase,
                      mods.value, inverted=inverted, touch_screen=device in (None, SCREEN))


def slide(vp, dx, dy, **kw):
    vp.wheelEvent(slide_event(dx, dy, **kw))


def finger_slide(vp, dx, dy, steps=5, **kw):
    """A whole slide: begin, ``steps`` updates of (dx, dy), end."""
    slide(vp, 0, 0, phase="start", **kw)
    for _ in range(steps):
        slide(vp, dx, dy, **kw)
    slide(vp, 0, 0, phase="end", **kw)


def notch(vp, dy, at=CENTRE):
    vp.wheelEvent(WheelEvent(at.x(), at.y(), 0, 0, 0, dy))


def pinch(vp, value, at=CENTRE):
    vp.event(NativeGestureEvent("zoom", value, at.x(), at.y(), touch_screen=True))


def gesture(vp, kind, value=0.0):
    vp.event(NativeGestureEvent(kind, value, CENTRE.x(), CENTRE.y(), touch_screen=True))


def pose(vp):
    c = vp.camera
    return (c.yaw, c.pitch, c.distance, c.target.x(), c.target.y(), c.target.z())


# ---- pure readers -----------------------------------------------------------

def test_slide_pixels_follow_the_fingers_with_natural_scrolling():
    assert trackpad_nav.slide_pixels(slide_event(3, -4, inverted=True)) == (3, -4)


def test_slide_pixels_are_a_wheels_without_natural_scrolling():
    assert trackpad_nav.slide_pixels(slide_event(3, -4, inverted=False)) == (-3, 4)


def test_slide_pixels_fall_back_to_the_angle_without_pixel_deltas():
    ev = WheelEvent(200, 150, 0, 0, 0, 120, "move", inverted=True, touch_screen=True)
    assert trackpad_nav.slide_pixels(ev) == (0, 15)


def test_a_mouse_notch_is_not_a_slide():
    ev = WheelEvent(200, 150, 0, 0, 0, 120)
    assert not trackpad_nav.is_slide(ev)
    assert trackpad_nav.is_slide(slide_event(0, 1))


@pytest.mark.parametrize("mods,expected", [
    (Qt.NoModifier, "orbit"), (Qt.ShiftModifier, "pan"),
    (Qt.ControlModifier, "zoom"), (Qt.ShiftModifier | Qt.ControlModifier, "pan")])
def test_modifiers_pick_the_slide_mode(mods, expected):
    assert trackpad_nav.slide_mode(slide_event(0, 1, mods=mods), "orbit") == expected


def test_an_unknown_preference_zooms():
    assert trackpad_nav.slide_mode(slide_event(0, 1), "fly") == "zoom"


def test_the_screen_and_the_trackpad_are_told_apart_by_the_device():
    assert trackpad_nav.is_touch_screen(slide_event(0, 1))
    assert not trackpad_nav.is_touch_screen(slide_event(0, 1, device=REAL_TRACKPAD))
    assert not trackpad_nav.is_touch_screen(slide_event(0, 1, device=MOUSE))


def test_primary_pointer_id_alone_does_not_identify_a_screen():
    device = SimpleNamespace(type=lambda: DeviceType.TouchPad, systemId=lambda: 1)
    assert not trackpad_nav.is_touch_screen(SimpleNamespace(device=lambda: device))


def test_a_pinch_of_one_notchs_scale_is_one_step():
    assert trackpad_nav.pinch_steps(1.0 / 0.9 - 1.0) == pytest.approx(1.0)
    assert trackpad_nav.pinch_steps(0.9 - 1.0) == pytest.approx(-1.0)
    assert trackpad_nav.pinch_steps(-1.0) is None


# ---- the slide ------------------------------------------------------------

def test_a_slide_orbits_and_keeps_the_distance(vp):
    yaw0, dist0 = vp.camera.yaw, vp.camera.distance
    finger_slide(vp, 10, 0)
    assert vp.camera.yaw != yaw0
    assert vp.camera.distance == pytest.approx(dist0)


def test_the_first_pixels_are_not_lost(vp):
    # Below the lock threshold nothing moves yet; once it is crossed the
    # whole slide so far is applied in one go.
    yaw0 = vp.camera.yaw
    slide(vp, 0, 0, phase="start")
    slide(vp, 2, 0)
    assert vp.camera.yaw == yaw0
    slide(vp, 2, 0)
    slide(vp, 2, 0)
    turned_in_3 = vp.camera.yaw - yaw0
    assert turned_in_3 != 0
    from tests.controller_surface import ControllerSurface as Viewport
    other = Viewport(None)
    other.resize(400, 300)
    other._touch_slide = "orbit"
    other._invert_orbit_y = False
    finger_slide(other, 6, 0, steps=1)
    assert other.camera.yaw - yaw0 == pytest.approx(turned_in_3)


def test_a_slide_grabs_the_model_like_a_drag(vp):
    # A mouse drag of the same pixels through the middle button convention.
    from tests.controller_surface import ControllerSurface as Viewport
    ref = Viewport(None)
    ref.resize(400, 300)
    pivot = ref._orbit_pivot_at(CENTRE.x(), CENTRE.y())
    ref.camera.orbit_about(pivot, 30, 10, ref.height())
    finger_slide(vp, 30, 10, steps=1)
    assert pose(vp) == pytest.approx(pose(ref))


def test_the_invert_orbit_preference_applies(vp):
    from tests.controller_surface import ControllerSurface as Viewport
    ref = Viewport(None)
    ref.resize(400, 300)
    ref._touch_slide = "orbit"
    ref._invert_orbit_y = False
    finger_slide(ref, 0, 10, steps=1)
    vp._invert_orbit_y = True
    finger_slide(vp, 0, -10, steps=1)
    assert pose(vp) == pytest.approx(pose(ref))


def test_shift_pans_without_turning(vp):
    yaw0, pitch0 = vp.camera.yaw, vp.camera.pitch
    t0 = pose(vp)[3:]
    finger_slide(vp, 20, 0, mods=Qt.ShiftModifier)
    assert (vp.camera.yaw, vp.camera.pitch) == (yaw0, pitch0)
    assert pose(vp)[3:] != pytest.approx(t0)


def test_ctrl_zooms_like_the_wheel(vp):
    d0 = vp.camera.distance
    # With natural scrolling the deltas follow the fingers: up is -y.
    finger_slide(vp, 0, -15, mods=Qt.ControlModifier)      # fingers up: in
    assert vp.camera.distance < d0


def test_the_zoom_preference_keeps_the_old_feel(vp):
    vp._touch_slide = "zoom"
    d0, yaw0 = vp.camera.distance, vp.camera.yaw
    finger_slide(vp, 0, -15)                                # fingers up: in
    assert vp.camera.distance < d0
    assert vp.camera.yaw == yaw0


def test_a_trackpad_slide_zooms_as_before_by_default(vp):
    d0, yaw0 = vp.camera.distance, vp.camera.yaw
    finger_slide(vp, 0, -15, device=REAL_TRACKPAD)          # fingers up: in
    assert vp.camera.distance < d0
    assert vp.camera.yaw == yaw0


def test_the_trackpad_can_be_told_to_orbit(vp):
    vp._trackpad_slide = "orbit"
    yaw0, d0 = vp.camera.yaw, vp.camera.distance
    finger_slide(vp, 10, 0, device=REAL_TRACKPAD)
    assert vp.camera.yaw != yaw0
    assert vp.camera.distance == pytest.approx(d0)


def test_the_pan_preference_pans(vp):
    vp._touch_slide = "pan"
    t0 = pose(vp)[3:]
    finger_slide(vp, 20, 0)
    assert vp.camera.yaw == pytest.approx(pose(vp)[0])
    assert pose(vp)[3:] != pytest.approx(t0)


def test_momentum_keeps_turning_with_inertia_on(vp):
    finger_slide(vp, 10, 0)
    yaw1 = vp.camera.yaw
    slide(vp, 6, 0, phase="momentum")
    assert vp.camera.yaw != yaw1


def test_inertia_off_drops_momentum(vp):
    from built_core.settings import settings as preferences
    settings = preferences()
    settings.setValue("nav/trackpad_inertia", "0")
    try:
        finger_slide(vp, 10, 0)
        yaw = vp.camera.yaw
        slide(vp, 6, 0, phase="momentum")
        assert vp.camera.yaw == yaw
    finally:
        settings.remove("nav/trackpad_inertia")


def test_a_new_slide_picks_its_pivot_again(vp):
    finger_slide(vp, 10, 0)
    first = trackpad_nav.gesture(vp).pivot
    assert first is not None
    slide(vp, 0, 0, phase="start")
    assert trackpad_nav.gesture(vp).pivot is None


def test_a_mouse_notch_still_zooms(vp):
    d0, yaw0 = vp.camera.distance, vp.camera.yaw
    notch(vp, 120)
    assert vp.camera.distance < d0
    assert vp.camera.yaw == yaw0


# ---- the pinch ------------------------------------------------------------

def test_spreading_the_fingers_zooms_in_and_pinching_zooms_out(vp):
    d0 = vp.camera.distance
    pinch(vp, 0.1)
    assert vp.camera.distance < d0
    d1 = vp.camera.distance
    gesture(vp, "end")
    pinch(vp, -0.1)
    assert vp.camera.distance > d1


def test_a_pinch_of_one_notchs_scale_zooms_one_notch(vp):
    from tests.controller_surface import ControllerSurface as Viewport
    wheel = Viewport(None)
    wheel.resize(400, 300)
    wheel._invert_wheel = False       # whatever another test left in QSettings
    notch(wheel, 120)
    pinch(vp, 1.0 / 0.9 - 1.0)
    assert vp.camera.distance == pytest.approx(wheel.camera.distance, rel=1e-6)


def test_tiny_pinch_steps_add_up_to_the_same_zoom(vp):
    # The first steps wait for the lock, then arrive all at once: nothing
    # of the gesture is lost.
    from tests.controller_surface import ControllerSurface as Viewport
    one = Viewport(None)
    one.resize(400, 300)
    pinch(one, 1.0 / 0.9 - 1.0)
    for _ in range(20):
        pinch(vp, (1.0 / 0.9) ** (1 / 20) - 1.0)
    assert vp.camera.distance == pytest.approx(one.camera.distance, rel=1e-4)


def test_the_point_under_the_fingers_stays_put(vp):
    at = QPointF(260, 110)
    focus = vp._world_under_cursor(at.x(), at.y())
    assert focus is not None
    d0 = vp.camera.distance
    for _ in range(10):
        pinch(vp, 0.04, at)
    assert vp.camera.distance < d0 * 0.75
    px = vp._world_to_pixel(focus)
    assert px is not None
    assert px[0] == pytest.approx(at.x(), abs=0.5)
    assert px[1] == pytest.approx(at.y(), abs=0.5)


def test_the_wheel_preference_does_not_flip_the_pinch(vp):
    vp._invert_wheel = True
    d0 = vp.camera.distance
    pinch(vp, 0.1)
    assert vp.camera.distance < d0


def test_a_pinch_in_progress_drops_the_slide_underneath(vp):
    pinch(vp, 0.1)
    p1 = pose(vp)
    slide(vp, 10, 0)                       # the fingers drifted sideways
    slide(vp, 10, 0)
    assert pose(vp) == p1


def test_a_slide_in_progress_drops_the_pinch_underneath(vp):
    finger_slide(vp, 10, 0)
    slide(vp, 0, 0, phase="start")
    slide(vp, 10, 0)
    d1 = vp.camera.distance
    pinch(vp, 0.2)                         # the fingers spread a little
    assert vp.camera.distance == d1


def test_lifting_the_fingers_frees_the_gesture(vp):
    pinch(vp, 0.1)
    gesture(vp, "end")
    yaw0 = vp.camera.yaw
    finger_slide(vp, 10, 0)
    assert vp.camera.yaw != yaw0


def test_a_rotation_orbits(vp):
    p0 = pose(vp)
    gesture(vp, "rotate", 15.0)
    assert vp.camera.yaw != p0[0]
    assert vp.camera.distance == pytest.approx(p0[2])


# ---- smart zoom (two-finger double tap) ------------------------------------

def smart_zoom(vp):
    gesture(vp, "smart_zoom", 1.0)


def test_a_smart_zoom_asks_the_window(vp):
    seen = []
    vp._host.window = SimpleNamespace(_on_smart_zoom=lambda: seen.append(1))
    smart_zoom(vp)
    assert seen == [1]


def test_a_double_tap_frames_the_model_and_a_second_goes_back(native_host):
    native_host.attach_surface()
    win, vp = native_host.window, native_host.viewport
    vp.scene.mesh.add_edge(QVector3D(0, 0, 0), QVector3D(20, 10, 6))
    vp.notify_scene_changed()
    vp.camera.distance = 2.0
    start = win._camera_pose()
    native_host.gesture("smart_zoom", 0., 350., 230.)
    framed = win._camera_pose()
    assert framed != start
    native_host.gesture("smart_zoom", 0., 350., 230.)
    assert win._camera_pose() == pytest.approx(start)


def test_after_moving_the_view_a_double_tap_frames_again(native_host):
    native_host.attach_surface()
    win, vp = native_host.window, native_host.viewport
    vp.scene.mesh.add_edge(QVector3D(0, 0, 0), QVector3D(20, 10, 6))
    vp.notify_scene_changed()
    vp.camera.distance = 2.0
    start = win._camera_pose()
    native_host.gesture("smart_zoom", 0., 350., 230.)
    framed = win._camera_pose()
    native_host.gesture("zoom", 0.2, 350., 230.)
    moved = win._camera_pose()
    native_host.gesture("smart_zoom", 0., 350., 230.)
    assert win._camera_pose() == pytest.approx(framed)
    assert win._camera_pose() != pytest.approx(start)
    assert win._camera_pose() != pytest.approx(moved)


# ---- the Metal view -------------------------------------------------------

def test_the_metal_view_orbits_and_pinches_too(native_host, monkeypatch):
    from types import SimpleNamespace

    from built_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D
    from built_core.group import Group
    from built_core.mesh import Mesh
    from built_core.scene import Scene
    from tests.controller_surface import ControllerSurface as Viewport
    from built_core import resident_prepare as prepare
    from built_core import pack_scene

    scene = Scene()
    mesh = Mesh()
    mesh.add_face([QVector3D(0, 0, 0), QVector3D(2, 0, 0),
                   QVector3D(2, 2, 0), QVector3D(0, 2, 0)])
    group = Group(mesh, name="placed")
    group.xform = QMatrix4x4()
    scene.groups.append(group)
    source = Viewport(None)
    source.resize(400, 300)
    source._touch_slide = "orbit"
    v = native_host.view
    native_host.resize(400, 300, 1.)
    v.source_viewport = source
    v.camera = source.camera
    v.scene = scene
    v.packed = pack_scene(scene)
    v.uid_rows = {uid: [i] for i, uid in enumerate(v.packed.uids)}
    v.camera.target = QVector3D(1, 1, 0)
    v.camera.distance = 8
    v.camera.aspect = 400 / 300
    v.inference_index = prepare(v.packed)
    v.renderer = SimpleNamespace(set_highlight=lambda *a: None,
                                 set_rest=lambda *a: None,
                                 pick=lambda *a: 0, pick_rect=lambda *a: [0])
    try:
        yaw0, d0 = v.camera.yaw, v.camera.distance
        finger_slide(v, 10, 0)
        assert v.camera.yaw != yaw0
        assert v.camera.distance == pytest.approx(d0)
        gesture(v, "end")
        pinch(v, 0.1)
        assert v.camera.distance < d0
        seen = []
        monkeypatch.setattr(native_host.window, "_on_smart_zoom", lambda: seen.append(1))
        smart_zoom(v)
        assert seen == [1]
    finally:
        v.renderer = None


def test_off_centre_navigation_pose_sequence(vp):
    """Pin yaw, pitch, distance and target at each input boundary."""
    vp.resize(640, 480)
    assert not vp.isVisible()
    vp.camera.aspect = 640 / 480
    at = QPointF(328, 196)
    actual = []
    finger_slide(vp, 10, -2, steps=3, at=at)
    actual.append(pose(vp))
    finger_slide(vp, -7, 4, steps=4, mods=Qt.ShiftModifier, at=at)
    actual.append(pose(vp))
    notch(vp, 120, at=at)
    actual.append(pose(vp))
    finger_slide(vp, 0, -1, steps=8, device=REAL_TRACKPAD, at=at)
    actual.append(pose(vp))
    slide(vp, 0, -2, phase="momentum", device=REAL_TRACKPAD, at=at)
    actual.append(pose(vp))
    pinch(vp, 0.14, at=at)
    actual.append(pose(vp))

    # Captured with the shared controller, in logical pixels and model metres.
    angles = (-0.9817477042468, 0.4843288674284)
    expected = [
        (20.0,               0.0,             0.0,             0.0),
        (20.0,               0.6607480921166, 0.7507715990379, 0.4887651759596),
        (18.0,               0.6444203731316, 0.8249120716871, 0.6231755993485),
        # Trackpad slide and inertia zoom at TRACKPAD_ZOOM_GAIN (3x, 2026-11-24);
        # the wheel notch above and the pinch's ratio below are unchanged.
        (15.2075943724250,   0.6216235659419, 0.9284272082153, 0.8108398106865),
        (14.58,              0.6164999736671, 0.9516922799171, 0.8530174233435),
        (12.7894736842105,   0.6018823684073, 1.0180675135888, 0.9733501234406),
    ]
    for observed, values in zip(actual, expected, strict=True):
        assert observed == pytest.approx((*angles, *values), rel=1e-10, abs=1e-10)
