"""File > Import exercises each survey backend against an isolated native document."""
import json
import zipfile

import pytest

from tests.shell_isolation import restore_shell_patches  # noqa: F401


@pytest.fixture
def host(monkeypatch, tmp_path):
    from ingetrazo_render import Host
    from georef.tile_fetcher import TileFetcher
    monkeypatch.setattr(TileFetcher, "request", lambda *_a, **_k: None)
    host = Host(0, 900, 600, 1., settings=str(tmp_path / "settings"))
    host.window.new_document()
    yield host
    host.close()


def pick(monkeypatch, path):
    from ingetrazo_render.shell_files import ShellFileDialog
    monkeypatch.setattr(ShellFileDialog, "getOpenFileName", lambda *_a, **_k: (str(path), ""))


@pytest.mark.parametrize("suffix", [".geojson", ".kml", ".kmz"])
def test_alignment_anchors_and_frames_the_import_then_undo(host, tmp_path, monkeypatch, suffix):
    path = tmp_path / ("route" + suffix)
    kml = '<kml><Placemark><LineString><coordinates>2,45 2.001,45.001</coordinates></LineString></Placemark></kml>'
    if suffix == ".geojson":
        path.write_text(json.dumps(dict(type="LineString", coordinates=[[2,45],[2.001,45.001]])))
    elif suffix == ".kmz":
        with zipfile.ZipFile(path,"w") as archive:
            archive.writestr("doc.kml", kml)
    else:
        path.write_text(kml)
    pick(monkeypatch, path)
    host.menu("import:alignment")
    scene = host.viewport.scene
    assert not host.errors
    assert (scene.georef.lat, scene.georef.lon) == pytest.approx((45.0005, 2.0005))
    assert len(scene.geo_paths) == 1 and len(scene.geo_paths[0].points) == 2
    assert scene.tile_layer is not None
    host.menu("undo")
    assert not scene.geo_paths


def test_survey_csv_utm_form_cancel_and_apply(host, tmp_path, monkeypatch):
    from ingetrazo_render import chrome_forms
    path = tmp_path / "survey.csv"
    path.write_text("A,5000000,500000,100,tree\nB,5000001,500002,103,curb\n")
    pick(monkeypatch, path)
    forms = []
    monkeypatch.setattr(chrome_forms, "show", lambda _win, form: forms.append(form))
    scene = host.viewport.scene
    host.menu("import:survey_points")
    form = forms.pop()
    form.on_done("cancel", {})
    assert scene.georef is None and not scene.geo_points
    host.menu("import:survey_points")
    form = forms.pop()
    form.model.receive("zone", "61")
    assert form.model.invalid_range() == (1,60)
    form.model.receive("zone", "32")
    form.model.receive("hemisphere", "1")
    form.on_done("ok", form.values())
    assert not host.errors
    assert scene.georef.zone == 32 and scene.georef.northern
    assert [p.name for p in scene.geo_points] == ["A", "B"]
    assert scene.geo_points[1].position.z() == pytest.approx(3.)
    host.menu("import:survey_points")
    assert len(scene.geo_points) == 4 and not forms
    host.menu("undo")
    host.menu("undo")
    assert not scene.geo_points and scene.georef is None


def test_odm_uses_survey_altitude_and_keeps_a_missing_atlas(host, tmp_path, monkeypatch):
    from tests.test_photomesh import _write_odm_tree
    pick(monkeypatch, _write_odm_tree(tmp_path))
    scene = host.viewport.scene
    host.menu("import:photomesh")
    assert not host.errors
    assert scene.georef.alt == pytest.approx(1750.)
    assert scene.photo_mesh.triangle_count == 2 and scene.photo_mesh.missing_textures
    assert scene.tile_layer is not None and scene.layer(scene.photo_mesh.layer) is not None
    assert host.window._is_dirty()


@pytest.mark.parametrize("outcome", ["error", "cancel"])
def test_odm_failure_or_cancel_preserves_unlocated_model(host, tmp_path, monkeypatch, outcome):
    from tests.test_photomesh import _write_odm_tree
    from ingetrazo_render import chrome_dialogs
    from ingetrazo_render.shell_files import NullBar
    pick(monkeypatch, _write_odm_tree(tmp_path))
    warnings = []
    monkeypatch.setattr(chrome_dialogs, "critical", lambda *args: warnings.append(args))
    bar = NullBar()
    monkeypatch.setattr(bar, "wasCanceled", lambda: outcome == "cancel")
    monkeypatch.setattr(host.window, "_import_progress", lambda _title: (bar, None))
    monkeypatch.setattr(host.window, "_load_photomesh_threaded", lambda *_: (None,None,ValueError("invalid OBJ")))
    host.menu("import:photomesh")
    assert not host.errors
    assert host.viewport.scene.georef is None and host.viewport.scene.photo_mesh is None
    assert bool(warnings) == (outcome == "error")
