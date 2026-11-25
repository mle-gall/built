# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Camera ▸ Camera Settings: the camera by the numbers — eye and target
typed in, the lens as a focal length, roll, frame shift, clipping and the
picture's frame — kept by scenes and by the document."""
from __future__ import annotations

from ingetrazo_render import open_camera
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from tests.shell_isolation import restore_shell_patches  # noqa: F401
import pytest  # noqa: E402
from ingetrazo_core import Input as Qt  # noqa: E402
from ingetrazo_core import Vec3 as QVector3D, Vec4 as QVector4D


from core.camera import (OrbitCamera, focal_from_fov,  # noqa: E402
                         fov_from_focal, frame_rect)
from ingetrazo_core.saved_views import SavedView  # noqa: E402


def _ndc(cam, p: QVector3D):
    mvp = cam.projection_matrix() * cam.view_matrix()
    v = mvp.map(QVector4D(p.x(), p.y(), p.z(), 1.0))
    return v.x() / v.w(), v.y() / v.w(), v.z() / v.w()


def _level_camera() -> OrbitCamera:
    """Eye 1.6 m up, 10 m south of a point at the same height, looking
    north: a level view of a facade."""
    cam = OrbitCamera()
    cam.set_aspect(1600, 900)
    assert cam.place(QVector3D(0, -10, 1.6), QVector3D(0, 0, 1.6))
    return cam


# ---- Lens arithmetic ---------------------------------------------------------

def test_focal_length_and_field_of_view_are_one_another():
    # 35 mm format by its 24 mm height: a 50 mm lens sees 27° vertically.
    assert fov_from_focal(50.0) == pytest.approx(26.99, abs=0.01)
    assert focal_from_fov(45.0) == pytest.approx(28.97, abs=0.01)
    for mm in (14.0, 24.0, 35.0, 85.0, 135.0):
        assert focal_from_fov(fov_from_focal(mm)) == pytest.approx(mm)
    cam = OrbitCamera()
    cam.fov_deg = fov_from_focal(35.0)
    assert cam.focal_mm == pytest.approx(35.0)


def test_an_ordinary_lens_draws_as_it_always_did():
    """No setting touched: the very same matrix as before the lens."""
    cam = _level_camera()
    assert cam.lens_dict() == {}
    from ingetrazo_core import Matrix4 as QMatrix4x4
    m = QMatrix4x4()
    m.perspective(cam.fov_deg, cam.aspect, cam.near_plane(), cam.far_plane())
    assert cam.projection_matrix() == m


def test_place_puts_the_eye_and_target_exactly():
    cam = OrbitCamera()
    eye, target = QVector3D(3.0, -7.5, 1.65), QVector3D(-2.0, 4.0, 1.2)
    assert cam.place(eye, target)
    assert (cam.eye() - eye).length() < 1e-5
    assert (cam.target - target).length() < 1e-5
    # The same point twice is no camera: refused, nothing moves.
    before = cam.eye()
    assert not cam.place(eye, eye)
    assert (cam.eye() - before).length() < 1e-9


# ---- Shift -------------------------------------------------------------------

def test_vertical_shift_slides_the_picture_and_keeps_verticals_upright():
    cam = _level_camera()
    cam.shift_y = 0.25
    # The point looked at sits half a screen below the centre…
    x, y, _ = _ndc(cam, QVector3D(0, 0, 1.6))
    assert x == pytest.approx(0.0, abs=1e-5)
    assert y == pytest.approx(-0.5, abs=1e-4)
    # …and a corner of a tall facade stays vertical on screen.
    a = _ndc(cam, QVector3D(4, 0, 0))
    b = _ndc(cam, QVector3D(4, 0, 12))
    assert a[0] == pytest.approx(b[0], abs=1e-5)


def test_horizontal_shift_and_shift_in_parallel():
    cam = _level_camera()
    cam.shift_x = -0.1
    x, _, _ = _ndc(cam, QVector3D(0, 0, 1.6))
    assert x == pytest.approx(0.2, abs=1e-4)
    cam.perspective = False
    x, _, _ = _ndc(cam, QVector3D(0, 0, 1.6))
    assert x == pytest.approx(0.2, abs=1e-4)


def test_shift_adds_to_a_two_point_rise():
    cam = _level_camera()
    cam.place(QVector3D(0, -10, 1.6), QVector3D(0, 0, 6.0))   # looking up
    cam.two_point = True
    _, y0, _ = _ndc(cam, cam.target)
    assert y0 == pytest.approx(0.0, abs=1e-4)                 # centred
    cam.shift_y = 0.1
    _, y1, _ = _ndc(cam, cam.target)
    assert y1 == pytest.approx(-0.2, abs=1e-4)


# ---- Roll ----------------------------------------------------------------------

def test_roll_turns_the_picture_about_the_sight_line():
    cam = _level_camera()
    cam.roll_deg = 90.0
    # Above the target is now beside it on screen, not above.
    x, y, _ = _ndc(cam, QVector3D(0, 0, 3.0))
    assert abs(y) < 1e-4 and abs(x) > 0.1
    # Panning follows the turned picture: a drag "up" moves along the roll.
    t = QVector3D(cam.target)
    cam.pan(0.0, 100.0, 900)
    moved = cam.target - t
    assert abs(moved.z()) < 1e-6 and abs(moved.x()) > 1e-3


def test_two_point_takes_no_roll():
    cam = _level_camera()
    cam.two_point = True
    cam.roll_deg = 30.0
    a = _ndc(cam, QVector3D(4, 0, 0))
    b = _ndc(cam, QVector3D(4, 0, 12))
    assert a[0] == pytest.approx(b[0], abs=1e-5)


# ---- Clipping ------------------------------------------------------------------

def test_near_clipping_looks_through_a_wall():
    cam = _level_camera()
    wall = QVector3D(0, -7, 1.6)          # 3 m in front of the eye
    assert -1.0 < _ndc(cam, wall)[2] < 1.0
    cam.clip_near = 4.0
    assert cam.near_plane() == pytest.approx(4.0)
    assert _ndc(cam, wall)[2] < -1.0      # in front of the near plane: gone
    assert -1.0 < _ndc(cam, QVector3D(0, 0, 1.6))[2] < 1.0


def test_far_clipping_and_clipping_in_parallel():
    cam = _level_camera()
    far = QVector3D(0, 30, 1.6)           # 40 m away
    cam.clip_far = 20.0
    assert _ndc(cam, far)[2] > 1.0
    cam.perspective = False
    cam.clip_near, cam.clip_far = 4.0, 20.0
    assert _ndc(cam, QVector3D(0, -7, 1.6))[2] < -1.0
    assert _ndc(cam, far)[2] > 1.0
    assert -1.0 < _ndc(cam, QVector3D(0, 0, 1.6))[2] < 1.0


def test_apply_lens_resets_and_cleans():
    cam = OrbitCamera()
    cam.apply_lens({"shift_y": 5.0, "roll_deg": 370.0, "clip_near": -1.0,
                    "clip_far": float("nan"), "frame_aspect": "x"})
    assert cam.shift_y == 1.0                 # at most a whole view
    assert cam.roll_deg == pytest.approx(10.0)
    assert cam.clip_near is None and cam.clip_far is None
    assert cam.frame_aspect is None
    cam.apply_lens({"clip_near": 5.0, "clip_far": 2.0})
    assert cam.clip_near == 5.0 and cam.clip_far is None   # far ≤ near dropped
    cam.apply_lens(None)
    assert cam.lens_dict() == {}


# ---- Frame ---------------------------------------------------------------------

def test_frame_is_the_largest_centred_rectangle():
    assert frame_rect(1600, 900, None) is None
    x, y, w, h = frame_rect(1600, 900, 1.0)
    assert (x, y, w, h) == (350.0, 0.0, 900.0, 900.0)
    x, y, w, h = frame_rect(1000, 1000, 2.0)
    assert (x, y, w, h) == (0.0, 250.0, 1000.0, 500.0)


# ---- Kept by scenes and by the document -------------------------------------------

def _Scene():
    from core.scene import Scene
    return Scene()


def test_a_scene_keeps_the_lens_and_an_old_scene_takes_it_off():
    cam = _level_camera()
    cam.apply_lens({"shift_y": 0.2, "roll_deg": -5.0, "clip_near": 3.0,
                    "frame_aspect": 16 / 9})
    view = SavedView.capture("Fachada", _Scene(), cam)
    raw = view.to_dict()
    assert raw["lens"]["shift_y"] == 0.2
    back = SavedView.from_dict(raw)
    other = OrbitCamera()
    back.apply(_Scene(), other)
    assert other.lens_dict() == cam.lens_dict()
    # A scene from before the lens (no "lens" key) is an ordinary lens.
    old = dict(raw)
    del old["lens"]
    SavedView.from_dict(old).apply(_Scene(), other)
    assert other.lens_dict() == {}


def test_the_document_keeps_the_lens(tmp_path, native_host):
    win = native_host.window
    cam = win.viewport.camera
    cam.apply_lens({"shift_y": 0.15, "frame_aspect": 4 / 3})
    home = win._camera_dict()
    assert home["lens"] == {"shift_y": 0.15, "frame_aspect": 4 / 3}
    cam.reset_lens()
    win._apply_camera_dict(home)
    assert cam.shift_y == 0.15 and cam.frame_aspect == pytest.approx(4 / 3)
    win._reset_camera_home()                   # File ▸ New
    assert cam.lens_dict() == {}


# ---- The window -------------------------------------------------------------------

def _edit(win, key, value=None):
    from ingetrazo_render import inspector_action as action
    import json

    view = getattr(win._doc, "stand_in", None) or win.viewport
    action(win, view, json.dumps(["camera." + key, value]))


def test_the_inspector_shows_and_sets_the_camera(native_host):
    win = native_host.window
    open_camera(win)
    cam = win.viewport.camera
    _edit(win, "projection", "perspective")
    for prefix, xyz in (("eye", (2., -12., 1.7)), ("target", (0., 0., 1.5))):
        for index, coordinate in enumerate(xyz):
            _edit(win, f"{prefix}{index}", coordinate)
    assert (cam.eye() - QVector3D(2, -12, 1.7)).length() < 1e-3
    assert (cam.target - QVector3D(0, 0, 1.5)).length() < 1e-3
    _edit(win, "focal", 50.)
    assert cam.fov_deg == pytest.approx(27, abs=.06)
    for key, value in (("shift_y", .2), ("roll_deg", 10.), ("clip_near", 3.), ("frame_aspect", 16/9)):
        _edit(win, key, value)
        assert getattr(cam, key) == pytest.approx(value)
    cam.place(QVector3D(5, 5, 5), QVector3D(0, 0, 0))
    _edit(win, "reset")
    assert cam.lens_dict() == {} and cam.focal_mm == pytest.approx(50)
    assert (cam.eye() - QVector3D(5, 5, 5)).length() < 1e-3
    _edit(win, "projection", "parallel")
    assert not cam.perspective
    _edit(win, "close")

def test_every_language_names_the_window():
    import json
    from ingetrazo_core.paths import app_root
    for p in (app_root() / "i18n").glob("*.json"):
        if p.stem == "en":
            continue
        data = json.loads(p.read_text(encoding="utf-8"))
        assert "Camera Settings…" in data, p.stem


# ---- Export image keeps to the frame ------------------------------------------------

def _fake_view(monkeypatch, w=1000, h=500):
    """A viewport whose render is a synthetic picture: grey all over and
    red exactly where the camera frame lies, so a crop can be checked
    without a graphics card."""
    import numpy as np
    from ingetrazo_render import SurveyRaster
    from views.document_controller import DocumentController
    cls = DocumentController
    vp = cls.__new__(cls)
    monkeypatch.setattr(cls, "width", lambda self: w)
    monkeypatch.setattr(cls, "height", lambda self: h)
    monkeypatch.setattr(cls, "update", lambda self: None)
    vp.camera = OrbitCamera()
    real = cls.render_raster
    rendered = []

    def render(self, width_px, height_px=None, overlays=True,
               annotations_only=False, sharp_shadows=False,
               transparent=False, crop_to_frame=False):
        if crop_to_frame:
            return real(self, width_px, height_px, overlays,
                        annotations_only, sharp_shadows, transparent, True)
        hh = round(width_px * h / w)
        rendered.append((width_px, hh))
        px = np.full((hh, width_px, 4), 128, np.uint8)
        px[..., 3] = 255
        rect = frame_rect(w, h, self.camera.frame_aspect)
        if rect is not None:
            k = width_px / w
            x, y = round(rect[0] * k), round(rect[1] * k)
            px[y:y + round(rect[3] * k), x:x + round(rect[2] * k)] = (255, 0, 0, 255)
        return SurveyRaster(width_px, hh, px.tobytes())

    monkeypatch.setattr(cls, "render_raster", render)
    return vp, rendered


def _pixel(img, x, y):
    import numpy as np
    return np.frombuffer(img.rgba(), np.uint8).reshape(img.height, img.width, 4)[y, x]


def test_export_image_is_the_framed_shot(monkeypatch):
    vp, rendered = _fake_view(monkeypatch)          # a 2:1 view
    vp.camera.frame_aspect = 1.0                    # a square frame
    img = vp.render_raster(800, crop_to_frame=True)
    # 800 px wide AND square: the frame, not the view.
    assert (img.width, img.height) == (800, 800)
    # Rendered twice as wide (the frame is half the view) and cut to it:
    # nothing outside the frame (grey) is left in the picture.
    assert rendered == [(1600, 800)]
    for x, y in ((0, 0), (799, 0), (0, 799), (799, 799), (400, 400)):
        assert _pixel(img, x, y)[0] == 255, (x, y)
        assert _pixel(img, x, y)[1] == 0, (x, y)


def test_export_image_tall_frame(monkeypatch):
    vp, rendered = _fake_view(monkeypatch, 800, 800)
    vp.camera.frame_aspect = 16 / 9                 # bands top and bottom
    img = vp.render_raster(1920, crop_to_frame=True)
    assert (img.width, img.height) == (1920, 1080)
    assert _pixel(img, 960, 2)[0] == 255
    assert _pixel(img, 960, 1077)[0] == 255


def test_export_image_menu_crops_to_the_frame(monkeypatch, tmp_path, native_host):
    from unittest.mock import Mock
    from ingetrazo_render import shell_services as native
    from ingetrazo_render import chrome_forms
    from tests.raster_samples import solid_raster
    from ingetrazo_render.shell_files import ShellFileDialog
    win = native_host.window
    render = Mock(return_value=solid_raster(16, 9, (0, 0, 0)))
    monkeypatch.setattr(native, "available", lambda: True)
    monkeypatch.setattr(win._active_view(), "render_raster", render)
    monkeypatch.setattr(ShellFileDialog, "getSaveFileName",
                        lambda *args: (str(tmp_path / "frame.png"), ""))
    win._on_export_image()
    chrome_forms.current(win).press("ok")
    render.assert_called_once_with(3840, sharp_shadows=True, transparent=False,
                                   crop_to_frame=True)
    assert (tmp_path / "frame.png").read_bytes().startswith(b"\x89PNG\r\n\x1a\n")


# ---- Basic and advanced ----------------------------------------------------------

def test_basic_settings_first_advanced_on_request(native_host):
    from ingetrazo_core.settings import settings
    win = native_host.window
    settings().remove("camera_settings/advanced")
    inspector = open_camera(win)
    assert not inspector.advanced
    _edit(win, "advanced", True)
    assert inspector.advanced
    _edit(win, "close")
    assert open_camera(win).advanced
    _edit(win, "advanced", False)
    assert not open_camera(win).advanced

def _window_with_scenes(win):
    scene = win.viewport.scene
    cam = win.viewport.camera
    cam.place(QVector3D(0, -10, 1.6), QVector3D(0, 0, 1.6))
    a = SavedView.capture("Fachada", scene, cam)
    cam.place(QVector3D(10, 10, 8), QVector3D(0, 0, 0))
    b = SavedView.capture("Aérea", scene, cam)
    scene.saved_views += [a, b]
    scene.version += 1
    return win, a, b


def test_right_click_a_scene_edits_its_camera(native_host):
    win, a, b = _window_with_scenes(native_host.window)
    scene, cam = win.viewport.scene, win.viewport.camera
    hidden = list(a.hidden_layers)
    version = scene.version
    inspector = win.edit_scene_camera(a)
    assert inspector.observe(cam, scene, a)
    # The scene is what the view shows.
    assert (cam.eye() - QVector3D(0, -10, 1.6)).length() < 1e-4
    # A change made here goes into the scene, and only the camera.
    _edit(win, "shift_y", .25)
    _edit(win, "focal", 24.)
    assert a.lens.get("shift_y") == pytest.approx(0.25)
    assert focal_from_fov(a.fov_deg) == pytest.approx(24.0, abs=0.01)
    assert a.hidden_layers == hidden
    assert scene.version > version            # the document changed
    assert b.lens == {}                       # the other scene: untouched
    # Moving the view by other means ends it: the scene keeps what it had.
    cam.orbit(80.0, 0.0, 900)
    _edit(win, "frame_aspect", 1.)
    assert not inspector.observe(cam, scene, a)
    assert "frame_aspect" not in a.lens
    # The Camera menu opens it on the live view, not on a scene.
    inspector = win.edit_scene_camera(b)
    assert inspector.observe(cam, scene, b)
    open_camera(win)
    assert not inspector.observe(cam, scene, b)
    _edit(win, "close")


def test_scene_menus_offer_camera_settings(native_host):
    from tests.scene_controls import choose
    from views.wgpu_ui import _run
    win = native_host.window
    for direction in ("front", "right"):
        win.viewport.camera.set_view(direction)
        win.run_command("scene_add")
    a, b = win.viewport.scene.saved_views
    choose(win, 1)
    _run(win, ["tab:scene:editcamera"])
    _edit(win, "shift_x", .125)
    assert b.lens["shift_x"] == .125 and not a.lens
    choose(win, 0)
    _run(win, ["tab:scene:editcamera"])
    _edit(win, "shift_y", .25)
    assert a.lens["shift_y"] == .25 and "shift_y" not in b.lens
    _edit(win, "close")


def test_take_camera_keeps_only_the_camera():
    from core.scene import Scene
    scene = Scene()
    cam = _level_camera()
    view = SavedView("Vista", hidden_layers=["Muebles"])
    cam.apply_lens({"roll_deg": 5.0})
    view.take_camera(cam)
    assert view.hidden_layers == ["Muebles"]
    assert view.lens == {"roll_deg": 5.0}
    other = OrbitCamera()
    view.apply(scene, other)
    assert (other.eye() - cam.eye()).length() < 1e-5


# ---- Whole on the screen, centred, still when the options open --------------------------

def test_replaced_scene_with_same_name_ends_writeback(native_host):
    win, a, b = _window_with_scenes(native_host.window)
    win.edit_scene_camera(a)
    scene, cam = win.viewport.scene, win.viewport.camera
    scene.saved_views[scene.saved_views.index(a)] = SavedView.capture(a.name, scene, cam)
    _edit(win, "shift_y", .2)
    assert not a.lens and not any(v.lens for v in scene.saved_views)

def test_a_picture_uses_the_largest_part_of_the_sensor_of_its_shape():
    from core.camera import FULL_FRAME, sensor_picture_height
    # Narrower than 3:2 (the usual view): the whole height.
    assert sensor_picture_height(FULL_FRAME, 1.4) == 24.0
    # 16:9 on a 3:2 sensor: the whole width, less height — as a camera films.
    assert sensor_picture_height(FULL_FRAME, 16 / 9) == pytest.approx(20.25)
    assert sensor_picture_height(None, None) == 24.0


def test_the_same_lens_sees_narrower_on_a_smaller_sensor():
    from core.camera import SENSORS
    cam = OrbitCamera()
    cam.set_aspect(1400, 1000)                     # narrower than 3:2
    ff = cam.fov_for_focal(50.0)
    cam.set_sensor(SENSORS["aps_c"])
    aps = cam.fov_for_focal(50.0)
    assert aps < ff
    # 50 mm on APS-C is what about 77 mm is on full frame (×1.54).
    cam.fov_deg = aps
    cam.set_sensor(None)
    assert cam.focal_mm == pytest.approx(50.0 * 24.0 / 15.6, rel=1e-6)
    # Changing the sensor keeps the picture: only the reading changes.
    cam.fov_deg = 40.0
    for size in SENSORS.values():
        cam.set_sensor(size)
        assert cam.fov_deg == 40.0
        assert cam.fov_for_focal(cam.focal_mm) == pytest.approx(40.0)


def test_the_sensor_is_kept_with_the_lens():
    from core.camera import SENSORS
    cam = OrbitCamera()
    assert "sensor" not in cam.lens_dict()           # full frame: nothing
    cam.set_sensor(SENSORS["micro_four_thirds"])
    assert cam.lens_dict()["sensor"] == [17.3, 13.0]
    other = OrbitCamera()
    other.apply_lens(cam.lens_dict())
    assert other.sensor_size() == (17.3, 13.0)
    # A scene keeps it, and an old scene goes back to full frame.
    from core.scene import Scene
    raw = SavedView.capture("V", Scene(), cam).to_dict()
    assert raw["lens"]["sensor"] == [17.3, 13.0]
    SavedView.from_dict(raw).apply(Scene(), other)
    assert other.sensor_size() == (17.3, 13.0)
    del raw["lens"]
    SavedView.from_dict(raw).apply(Scene(), other)
    assert other.sensor_size() == (36.0, 24.0)
    # Nonsense is not taken; Reset Lens keeps the sensor.
    other.apply_lens({"sensor": [0, 5]})
    assert other.sensor is None
    other.apply_lens({"sensor": "x", "roll_deg": 5})
    assert other.sensor is None and other.roll_deg == 5
    cam.roll_deg = 10.0
    cam.reset_lens()
    assert cam.roll_deg == 0.0 and cam.sensor_size() == (17.3, 13.0)


def test_the_inspector_picks_the_sensor(native_host):
    win = native_host.window
    model = open_camera(win)
    cam = win.viewport.camera
    cam.fov_deg = 40.
    original = cam.focal_mm
    _edit(win, "sensor", [23.6, 15.6])
    assert cam.sensor_size() == (23.6, 15.6)
    assert cam.fov_deg == 40. and cam.focal_mm < original
    _edit(win, "focal", 50.)
    assert cam.focal_mm == pytest.approx(50.)
    _edit(win, "sensor", None)
    for size in ((36., 24.), (30., 20.)):
        _edit(win, "sensor0", size[0])
        _edit(win, "sensor1", size[1])
        assert cam.sensor_size() == size and model.custom_sensor
    _edit(win, "close")
    assert not open_camera(win).custom_sensor and cam.sensor_size() == (30., 20.)

def _type(host, text, commit=True):
    for ch in text:
        host.key(True, ord(ch.upper()), text=ch)
    if commit:
        host.key(True, Qt.Key_Return, text="\r")


def test_typing_the_lens_with_zoom_on(native_host):
    win = native_host.window
    vp = win.viewport
    cam = vp.camera
    cam.set_projection("perspective")
    vp.set_nav_mode("zoom")
    _type(native_host, "35mm")
    assert cam.focal_mm == pytest.approx(35.0, abs=0.01)
    _type(native_host, "60deg")
    assert cam.fov_deg == pytest.approx(60.0)
    _type(native_host, "45")                               # bare: degrees
    assert cam.fov_deg == pytest.approx(45.0)
    _type(native_host, "72,5°")
    assert cam.fov_deg == pytest.approx(72.5)
    # The sensor counts: 35mm on APS-C is a narrower view.
    from core.camera import SENSORS
    cam.set_sensor(SENSORS["aps_c"])
    _type(native_host, "35mm")
    assert cam.focal_mm == pytest.approx(35.0, abs=0.01)
    assert cam.fov_deg < OrbitCamera().fov_for_focal(35.0)
    # Out of range: held to 10°–120°; nonsense changes nothing.
    _type(native_host, "500deg")
    assert cam.fov_deg == 120.0
    before = cam.fov_deg
    assert not vp.apply_zoom_value("abc")
    assert cam.fov_deg == before
    # Letters before a digit are not taken (they stay shortcuts).
    assert vp._value_buffer == ""
    # A parallel view has no lens: nothing changes.
    cam.set_projection("parallel")
    _type(native_host, "50mm")
    assert cam.fov_deg == before
    # Without Zoom, the value box is the tools' as before.
    cam.set_projection("perspective")
    vp.set_nav_mode(None)

    from ingetrazo_render import KeyEvent
    five = KeyEvent(True, Qt.Key_5, 0, "5", False)
    assert not vp._handle_value_key(five)
    assert vp._value_buffer == ""


def test_the_zoom_hint_says_what_to_type():
    from ingetrazo_render.status_hints import NAV_HINTS
    assert "35mm" in NAV_HINTS["zoom"] and "60deg" in NAV_HINTS["zoom"]


def test_the_value_box_shows_with_zoom(native_host):
    from tests.scene_controls import chrome
    from views.wgpu_ui import _cursor_value
    win = native_host.window
    native_host.pointer("move", 250, 180)
    win._activate_nav("zoom")
    def value():
        with chrome(win) as renderer:
            _cursor_value(win.view, win)
            return renderer.ui_set_cursor_value.call_args.args[1:3]
    _type(native_host, "35mm", commit=False)
    assert value() == ("35mm", True)
    native_host.key(True, Qt.Key_Return)
    assert value() == ("35 mm", False)
    win._activate_nav("orbit")
    assert value() == ("", False)


def test_open_and_close_camera_keep_the_model_interactive(native_host):
    win, a, _ = _window_with_scenes(native_host.window)
    inspector = win.edit_scene_camera(a)
    assert not native_host._modal()
    _edit(win, "close")
    _edit(win, "fov", 99.)
    assert not inspector.observe(win.viewport.camera, win.viewport.scene, a)
    assert win.viewport.camera.fov_deg != 99. and a.fov_deg != 99.

def test_from_24_to_35_typing_with_a_space_and_capitals(native_host):
    win = native_host.window
    vp = win.viewport
    cam = vp.camera
    cam.set_projection("perspective")
    cam.fov_deg = cam.fov_for_focal(24.0)
    assert win.run_command("zoom")
    _type(native_host, "35 MM", commit=False)
    assert vp.nav_mode == "zoom" and win.view.tools.value_buffer.lower() == "35 mm"
    native_host.key(True, Qt.Key_Return)
    assert cam.focal_mm == pytest.approx(35., abs=.01)
    _type(native_host, "533mm")
    assert cam.focal_mm == pytest.approx(533., abs=.1)
    assert cam.fov_deg == pytest.approx(2.58, abs=.01)
    _type(native_host, "60deg")
    assert cam.fov_deg == pytest.approx(60.) and vp.nav_mode == "zoom"


def test_after_picking_a_scene_z_then_35mm_works(native_host):
    from tests.scene_controls import choose
    win, a, _ = _window_with_scenes(native_host.window)
    choose(win, 0)
    assert win._scene_tabs.marked_view() is a
    for inspect, mm in ((False, 35.), (True, 50.)):
        if inspect:
            open_camera(win)
        native_host.key(True, Qt.Key_Z, text="z")
        assert win.viewport.nav_mode == "zoom"
        _type(native_host, f"{int(mm)}mm")
        assert win.viewport.camera.focal_mm == pytest.approx(mm, abs=.01)
        win._activate_nav("orbit")
    _edit(win, "close")
