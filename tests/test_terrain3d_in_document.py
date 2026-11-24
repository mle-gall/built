# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The 3D terrain is the document's: saved with its base map, back on
when the file is opened again, its own in each tab. It lived in the
window, so a project saved with the terrain on reopened flat after a
restart, and what stood on the terrain floated over the map."""
from __future__ import annotations

import pytest

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_settings_sheet import win, fields, frame, act  # noqa: F401
from tests.test_terrain import StubSampler
from georef.datum import SceneDatum


def _georef_scene(terrain3d=None):
    from core.scene import Scene
    from georef.tiles import PRESETS, TileLayer
    scene = Scene()
    scene.georef = SceneDatum(-16.39889, -71.53694)
    layer = TileLayer(PRESETS["esri_imagery"], zoom=16)
    layer.set_rectangle(400.0, 300.0)
    if terrain3d is not None:
        layer.terrain3d = terrain3d
    scene.tile_layer = layer
    return scene


def test_the_flag_round_trips_and_old_files_open_flat(tmp_path):
    from core.scene import Scene
    from formats import igz
    for want in (True, False):
        out = tmp_path / f"t{want}.igz"
        igz.save_scene(_georef_scene(want), out)
        back = Scene()
        igz.load_into(back, out)
        assert back.tile_layer.terrain3d is want
    from georef.tiles import TileLayer
    raw = _georef_scene().tile_layer.to_dict()
    del raw["terrain3d"]                       # a file from before
    assert TileLayer.from_dict(raw).terrain3d is False


@pytest.fixture
def dem(monkeypatch):
    from georef import dem
    from ingetrazo_render import ChangeCallbacks
    from views.document_controller import DocumentController

    class LocalDEM(StubSampler):
        def __init__(self, datum):
            super().__init__(datum, lambda x, y: 100.0 + x / 10.0)
            self.changed = ChangeCallbacks()
            self.areas = []
            self._host = None

        def ensure_area(self, *area):
            self.areas.append(area)

    monkeypatch.setattr(dem, "DEMSampler", LocalDEM)
    monkeypatch.setattr(DocumentController, "prefetch_tiles", lambda *args: None)


def test_turning_it_on_is_kept_in_the_document(win, dem):
    vp = win.viewport
    model = _georef_scene()
    vp.scene.georef, vp.scene.tile_layer = model.georef, model.tile_layer
    win.open_preferences(page="location")
    assert fields(frame(win))["terrain3d"][4] == 0
    act(win, "v", "terrain3d", "1")
    assert vp.scene.tile_layer.terrain3d and win._is_dirty()
    assert vp.scene.terrain.height_at(100, 0) == pytest.approx(10.0)
    assert vp._surface_sampler._host is vp._host and vp._surface_sampler.areas
    act(win, "v", "terrain3d", "0")
    assert vp.scene.terrain is None and not vp.scene.tile_layer.terrain3d
    vp._surface_sampler.changed.emit()
    assert vp.scene.terrain is None


def _open(host, path, relief, monkeypatch):
    from formats import igz
    igz.save_scene(_georef_scene(relief), path)
    win = host.window
    assert win.open_path(path)
    vp = win.viewport
    assert vp.scene.terrain is None
    monkeypatch.setattr(host.view, "draw_frame", lambda: (0., 0., 0.))
    host.frame()
    while host._after_picture:
        host.idle()
    return vp


def test_a_file_saved_in_3d_opens_in_3d(native_host, tmp_path, monkeypatch, dem):
    vp = _open(native_host, tmp_path / "relief.igz", True, monkeypatch)
    assert vp.scene.tile_layer.terrain3d
    assert vp.scene.terrain.bbox == (-200, -150, 200, 150)
    assert not native_host.window._is_dirty()
    vp.scene.terrain.visible = False
    vp.camera.set_view("top")
    pitch = vp.camera.pitch
    vp._surface_sampler.changed.emit()
    assert not vp.scene.terrain.visible and vp.camera.pitch == pitch


def test_a_file_saved_flat_opens_flat(native_host, tmp_path, monkeypatch, dem):
    first = _open(native_host, tmp_path / "hills.igz", True, monkeypatch)
    vp = _open(native_host, tmp_path / "plain.igz", False, monkeypatch)
    first._surface_sampler.changed.emit()
    assert vp is not first and first.scene.terrain is not None
    assert not vp.scene.tile_layer.terrain3d and vp.scene.terrain is None
    assert not hasattr(vp, "_surface_sampler")


def test_a_new_capture_keeps_the_terrain_as_it_was(win, dem):
    from ingetrazo_core import Vec3
    scene = win.viewport.scene
    scene.georef = SceneDatum(-16.39889, -71.53694)
    def capture(size):
        win._set_survey_capture(scene.georef, Vec3(-size, -size, 0), Vec3(size, size, 0))
    capture(50)
    win.open_preferences(page="location")
    assert fields(frame(win))["terrain3d"][4] == 0
    act(win, "v", "terrain3d", "1")
    original = scene.terrain.bbox
    capture(500)
    assert scene.tile_layer.terrain3d and scene.terrain.bbox[2] > original[2]
