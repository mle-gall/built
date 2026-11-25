# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Render tab and its lights (#181; Marco: «aprovechar la barra lateral
derecha para el apartado renderizar… lo único que le falta es puntos de
luz o focos, como para renderizar de noche»). Render with Blender lives in
the side tray, not a menu; point lights and spots are clicked onto the
model, kept in the document (undoable, saved in the .igz) and become real
Blender lights; an ambience of day, night or overcast sets the sky."""
from __future__ import annotations

import json
import shutil
import subprocess

import pytest
from ingetrazo_core import ScreenPoint as QPointF, Input as Qt
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_core import Vec3 as V

from core import render_blender as rb  # noqa: E402
from core.camera import OrbitCamera  # noqa: E402
from core.scene import Scene  # noqa: E402


# ---- The data ----------------------------------------------------------------

def test_clean_lights_keeps_only_well_formed_ones():
    raw = [
        {"kind": "point", "pos": [1, 2, 3], "color": "cool", "power": 250},
        {"kind": "spot", "pos": [0, 0, 5], "dir": [0, 0, 0], "angle": 500},
        {"kind": "laser", "pos": [0, 0, 0]},             # unknown kind
        {"kind": "point", "pos": ["x", 0, 0]},           # not a number
        {"kind": "point", "pos": [float("nan"), 0, 0]},  # not finite
        "garbage",
    ]
    out = rb.clean_lights(raw)
    assert [lt["kind"] for lt in out] == ["point", "spot"]
    assert out[0]["color"] == list(rb.LIGHT_COLORS["cool"])
    assert out[1]["dir"] == [0.0, 0.0, -1.0]             # a null aim → down
    assert out[1]["angle"] == 179.0                      # clamped
    assert out[1]["power"] == rb.DEFAULT_POWER["spot"]
    assert rb.clean_lights(None) == []


def _box():
    scene = Scene()
    m = scene.mesh
    p = [V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0),
         V(0, 0, 2), V(2, 0, 2), V(2, 2, 2), V(0, 2, 2)]
    for loop in ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4),
                 (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)):
        m.add_face([p[i] for i in loop])
    return scene


def _cam(scene):
    cam = OrbitCamera()
    cam.set_aspect(320, 180)
    cam.yaw, cam.pitch = -1.0, 0.4
    lo, hi = scene.bounds()
    cam.fit_to(lo, hi)
    return cam


def test_night_and_overcast_have_no_sun_and_lights_off_stay_out(tmp_path):
    scene = _box()
    lights = [{"kind": "point", "pos": [1, 1, 3], "on": True},
              {"kind": "spot", "pos": [1, 1, 5], "on": False}]
    for amb in rb.AMBIENCES:
        job = json.loads(rb.write_job(scene, _cam(scene), tmp_path / amb,
                                      ambience=amb, lights=lights)
                         .read_text())
        assert job["ambience"] == amb
        assert job["exposure"] == rb.EXPOSURE[amb]
        assert len(job["lights"]) == 1 and job["lights"][0]["kind"] == "point"
        if amb != "day":
            assert job["sun"] is None
    with pytest.raises(ValueError):
        rb.write_job(scene, _cam(scene), tmp_path / "x", ambience="dusk")


# ---- The panel -----------------------------------------------------------------

@pytest.fixture
def window(native_host):
    return native_host.window


def _panel(win):
    from views.extension_api import ExtensionApp
    return win._panel_factories["render_blender"](ExtensionApp(win, "render_blender"), win.viewport.update)


def test_the_extensions_entry_brings_back_the_render_tab(window, monkeypatch):
    view = attach_render_surface(window, monkeypatch)
    window._host.menu("panel:render_blender")
    panel = next(b for b in view._plugin_bindings.values() if b.key == "render_blender")
    view.renderer.ui_plugin_open.assert_called_with(panel.instance)


def test_a_light_clicked_onto_the_model_is_kept_and_undone(window, tmp_path):
    panel = _panel(window)
    vp = window.viewport
    panel.event("spot", "invoke", None)
    tool = vp.active_tool
    assert tool.uses_snap
    from core.snap import SnapResult
    from tools.base import ToolContext
    tool.on_click(ToolContext(viewport=vp, world=V(3, 4, 5.2),
                              screen=QPointF(0, 0), modifiers=Qt.NoModifier,
                              snap=SnapResult(V(3, 4, 5.2), "endpoint",
                                              (0, 0, 0))))
    lights = panel.app.document_data({})["lights"]
    assert len(lights) == 1
    assert lights[0]["kind"] == "spot"
    assert lights[0]["pos"] == pytest.approx([3, 4, 5.2])
    assert lights[0]["dir"] == [0.0, 0.0, -1.0]
    assert type(vp.active_tool).__name__ != type(tool).__name__  # back to Select
    assert panel.selected == "0"
    # Saved with the document, ambience included.
    panel.event("ambience", "commit", "night")
    path = tmp_path / "noche.igz"
    from formats import igz
    igz.save_scene(vp.scene, path)
    again = Scene()
    igz.load_into(again, path)
    data = again.plugin_data["render_blender"]
    assert data["ambience"] == "night" and len(data["lights"]) == 1
    # Undo: the ambience, then the light.
    vp.history.undo()
    vp.history.undo()
    assert not (panel.app.document_data({}) or {}).get("lights")


def test_a_point_light_clicked_on_a_face_sits_a_hand_above_it(window):
    panel = _panel(window)
    panel.add_light("point", V(1, 1, 0), on_face=True)
    lt = panel.app.document_data({})["lights"][-1]
    assert lt["pos"][2] == pytest.approx(0.1)


def test_esc_while_placing_cancels(window):
    panel = _panel(window)
    vp = window.viewport
    panel.event("point", "invoke", None)
    tool = vp.active_tool
    assert tool.on_key(vp, Qt.Key_Escape, Qt.NoModifier) is True
    assert vp.active_tool is not tool
    assert not (panel.app.document_data({}) or {}).get("lights")


@pytest.mark.slow
@pytest.mark.skipif(shutil.which("blender") is None,
                    reason="no Blender on this machine")
def test_a_night_render_with_lights(tmp_path):
    scene = _box()
    job = rb.write_job(scene, _cam(scene), tmp_path, engine="eevee",
                       quality=0, width=160, height=90, ambience="night",
                       lights=[{"kind": "spot", "pos": [1, 1, 5]},
                               {"kind": "point", "pos": [3, 1, 1]}])
    found = rb.BlenderFound([shutil.which("blender")], "blender")
    run = subprocess.run(rb.command(found, job), capture_output=True,
                         text=True, env=rb.clean_env(), timeout=600)
    assert run.returncode == 0, run.stdout[-2000:]
    assert "INGETRAZO sky night" in run.stdout
    assert "INGETRAZO lights 2" in run.stdout
    assert (tmp_path / "render.png").is_file()


def test_the_lights_are_drawn_over_the_viewport(window):
    """The overlay paints a bulb at each light (an overlay that raised would
    only be logged and skipped, so draw it here and look at the pixels)."""
    from ingetrazo_render import AnnotationCanvas
    from views.wgpu_overlay import paint_layer
    panel = _panel(window)
    vp = window.viewport
    window._host.resize(400, 300, 1.0)
    vp.camera.target = V(0, 0, 0)
    vp.camera.distance = 10
    panel.add_light("spot", V(0, 0, 1))
    canvas = AnnotationCanvas(400, 300, 400.0)
    paint_layer(canvas, list(panel.overlay(vp)))
    rgba = canvas.composite(bytes(400 * 300 * 4))
    x, y = vp._world_to_pixel(V(0, 0, 1))
    assert rgba[(int(y) * 400 + int(x)) * 4 + 3] > 0


# ---- Temperature, aim, the sun, the image window (Marco's second round) ------

def test_temperature_is_a_range_and_old_names_still_read():
    warm, day, sky = (rb.kelvin_to_rgb(k) for k in (2700, 6500, 10000))
    assert warm[0] == 1.0 and warm[2] < 0.5          # orange
    assert min(day) > 0.95                           # almost white
    assert sky[2] == 1.0 and sky[0] < 0.85           # bluish
    old = rb.clean_lights([{"kind": "point", "pos": [0, 0, 0],
                            "color": "neutral"}])[0]
    assert old["kelvin"] == 4000
    new = rb.clean_lights([{"kind": "point", "pos": [0, 0, 0],
                            "kelvin": 99999}])[0]
    assert new["kelvin"] == rb.MAX_KELVIN            # clamped


@pytest.mark.parametrize("heading, tilt", [(0, 90), (90, 45), (225, 30),
                                           (310, -20)])
def test_a_spot_aims_by_heading_and_tilt(heading, tilt):
    d = rb.aim_from_angles(heading, tilt)
    h, t = rb.angles_from_aim(d)
    assert t == pytest.approx(tilt, abs=0.1)
    if tilt != 90:
        assert h == pytest.approx(heading, abs=0.1)


def test_the_panel_sets_temperature_aim_and_sun(window):
    panel = _panel(window)
    panel.add_light("spot", V(0, 0, 4))
    panel.event("lights", "selection", "0")
    panel.event("kelvin", "commit", 5000)
    panel.event("tilt", "commit", 45)
    panel.event("heading", "commit", 90)
    lt = panel.app.document_data({})["lights"][0]
    assert lt["kelvin"] == 5000
    assert lt["dir"] == pytest.approx(rb.aim_from_angles(90, 45))
    panel.event("sun_scale", "commit", 1.5)
    assert panel.app.document_data({})["sun_scale"] == 1.5


def attach_render_surface(window, monkeypatch):
    from unittest.mock import Mock
    view = window.view
    monkeypatch.setattr(view, "renderer", Mock())
    monkeypatch.setattr(view, "_ui_attached", True, raising=False)
    return view


def test_a_finished_render_reuses_the_image_utility(window, tmp_path, monkeypatch):
    from tests.plugin_images import write_png
    view = attach_render_surface(window, monkeypatch)
    panel = _panel(window)
    panel.image = tmp_path / "render.png"
    write_png(panel.image, 64, 40)
    panel._show_result()
    first = panel.viewer
    assert first.model.slot == "utility" and first.model.path == str(panel.image)
    panel._show_result()
    assert panel.viewer is first and view.renderer.ui_plugin_open.call_count == 2
    assert view.renderer.ui_plugin_open.call_args.args == (first.instance,)


def test_the_sun_strength_reaches_the_job(tmp_path):
    scene = _box()
    job = json.loads(rb.write_job(scene, _cam(scene), tmp_path,
                                  sun_scale=0.5).read_text())
    assert job["sun_strength"] == pytest.approx(1.5)


# ---- The tray layout (Marco's third round) --------------------------------------

def test_sections_fold_and_remember_it(window):
    panel = _panel(window)
    panel.event("lights_open", "commit", False)
    again = _panel(window)
    def controls():
        return {c["id"] for section in again.sections() for c in section["items"]}
    assert "lights_open" in controls() and "lights" not in controls()
    again.event("lights_open", "commit", True)
    assert "lights" in controls()


def test_save_and_open_folder_live_in_the_image_utility(window, tmp_path, monkeypatch):
    from tests.plugin_images import write_png
    from unittest.mock import Mock
    attach_render_surface(window, monkeypatch)
    panel = _panel(window)
    panel.image = tmp_path / "render.png"
    write_png(panel.image, 32, 20)
    folder = Mock()
    from plugins import _blender_panel
    from views.filedialogs import file_dialogs
    saved = tmp_path / "saved.png"
    monkeypatch.setattr(file_dialogs, "getSaveFileName", lambda *_: (str(saved), ""))
    monkeypatch.setattr(_blender_panel, "open_url", folder)
    panel._show_result()
    model = panel.viewer.model
    controls = {i["id"] for s in model.sections() for i in s["items"]}
    assert {"image", "save", "folder", "follow"} <= controls
    model.event("save", "invoke", None)
    model.event("folder", "invoke", None)
    assert saved.read_bytes() == panel.image.read_bytes()
    folder.assert_called_once_with(tmp_path)


def test_change_date_and_time_drops_the_shadows_panel(window, monkeypatch):
    from unittest.mock import Mock
    run = Mock()
    monkeypatch.setattr(window, "run_command", run)
    panel = _panel(window)
    assert any(i["id"] == "shadows" for section in panel.sections() for i in section["items"])
    panel.event("shadows", "invoke", None)
    run.assert_called_once_with("view:form-shadows")
