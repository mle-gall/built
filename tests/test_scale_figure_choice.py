# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The scale figure of new documents can be any person, or the user's own
picture (#655) — and whatever the app placed stays out of a document
inserted into another, which only recognised Sumari until now."""
from __future__ import annotations


import pytest  # noqa: E402
from ingetrazo_core import NativeSettings
from core.settings import settings as preferences
from tests.shell_isolation import restore_shell_patches  # noqa: F401
import numpy as np
from ingetrazo_render import SurveyRaster
from ingetrazo_core import Vec3 as V

from core import scale_figure as sf  # noqa: E402
from core.group import Group, make_billboard_group  # noqa: E402
from core.mesh import Mesh  # noqa: E402


@pytest.fixture
def settings(tmp_path, monkeypatch):
    st = NativeSettings(str(tmp_path))
    monkeypatch.setattr(sf, "figures_dir", lambda: tmp_path / "figures")
    return st


def _png(path, alpha=True, w=40, h=120):
    data = np.full((h, w, 4), 0 if alpha else 255, np.uint8)
    data[10:, 10:w-10] = (0, 0, 128, 255)
    assert SurveyRaster(w, h, data.tobytes()).save(path)
    return path


def test_the_engineer_by_default_and_none_when_switched_off(settings):
    fig = sf.chosen(settings)
    assert fig.image.name == "ingeniero.png" and fig.height == pytest.approx(1.70)
    settings.setValue(sf.KEY_ON, "0")
    assert sf.chosen(settings) is None


def test_any_person_of_the_library_with_its_real_height(settings):
    settings.setValue(sf.KEY_PICK, "stallman")
    fig = sf.chosen(settings)
    assert fig.image.name == "stallman.png"
    assert fig.height == pytest.approx(1.78)


def test_my_own_picture_is_copied_and_survives_the_original(settings, tmp_path):
    src = _png(tmp_path / "yo.png")
    stored = sf.store_picture(src)
    assert stored.parent == tmp_path / "figures" and stored.is_file()
    assert sf.store_picture(src) == stored             # same bytes, one file
    src.unlink()                                       # the original goes
    settings.setValue(sf.KEY_PICK, sf.CUSTOM)
    settings.setValue(sf.KEY_IMAGE, str(stored))
    settings.setValue(sf.KEY_HEIGHT, 1.62)
    settings.setValue(sf.KEY_NAME, "Marco")
    fig = sf.chosen(settings)
    assert (fig.image, fig.height, fig.name) == (stored, pytest.approx(1.62), "Marco")


def test_a_lost_picture_falls_back_to_the_engineer_not_to_nothing(settings, tmp_path):
    settings.setValue(sf.KEY_PICK, sf.CUSTOM)
    settings.setValue(sf.KEY_IMAGE, str(tmp_path / "gone.png"))
    assert sf.chosen(settings).image.name == "ingeniero.png"
    settings.setValue(sf.KEY_PICK, "nobody")
    assert sf.chosen(settings).image.name == "ingeniero.png"


def test_a_file_that_is_not_a_picture_is_refused(settings, tmp_path):
    bad = tmp_path / "x.png"
    bad.write_bytes(b"not an image")
    with pytest.raises(ValueError):
        sf.store_picture(bad)
    assert not (tmp_path / "figures").exists()


def test_the_height_is_kept_within_sense(settings, tmp_path):
    stored = sf.store_picture(_png(tmp_path / "a.png"))
    settings.setValue(sf.KEY_PICK, sf.CUSTOM)
    settings.setValue(sf.KEY_IMAGE, str(stored))
    settings.setValue(sf.KEY_HEIGHT, 170)              # cm typed as m
    assert sf.chosen(settings).height == pytest.approx(sf.MAX_HEIGHT)


def test_a_new_document_stands_my_picture_and_marks_it(native_host, settings, tmp_path):
    stored = sf.store_picture(_png(tmp_path / "yo.png"))
    st = preferences()
    st.setValue(sf.KEY_PICK, sf.CUSTOM)
    st.setValue(sf.KEY_IMAGE, str(stored))
    st.setValue(sf.KEY_HEIGHT, 1.62)
    st.setValue(sf.KEY_NAME, "Marco")
    native_host.cold_start()
    win = native_host.window
    figs = [g for g in win.viewport.scene.groups if sf.is_marked(g)]
    assert len(figs) == 1 and figs[0].name == "Marco"
    fig = figs[0]
    zs = [v.position.z() for v in fig.mesh.vertices]
    assert max(zs) - min(zs) == pytest.approx(1.62, abs=1e-4)
    assert fig.mesh.faces[0].attrs["texture"]["path"] == str(stored)
    assert not win._is_dirty()


def test_the_mark_travels_in_the_igz(tmp_path):
    from core.scene import Scene
    from formats import igz
    scene = Scene()
    g = make_billboard_group(str(_png(tmp_path / "p.png")), 1.7, "P", 0.33)
    sf.mark(g)
    scene.groups.append(g)
    igz.save_scene(scene, tmp_path / "d.igz")
    back = Scene()
    igz.load_into(back, tmp_path / "d.igz")
    assert sf.is_marked(back.groups[0])


def test_inserting_a_document_leaves_out_whatever_figure_the_app_placed(tmp_path):
    from core.insert import is_scale_figure
    mine = make_billboard_group(str(_png(tmp_path / "yo.png")), 1.6, "Marco", 0.33)
    sf.mark(mine)
    engineer_old = make_billboard_group("/x/c1-ingeniero.png", 1.7, "Ingeniero", 0.4)
    sumari_old = make_billboard_group("/x/928c-sumari.png", 1.68, "Group 1", 0.35)
    placed_on_purpose = make_billboard_group("/x/stallman.png", 1.78, "Richard", 0.4)
    toro = Group(Mesh(), name="Toro")
    toro.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 0, 1), V(0, 0, 1)])
    toro.mesh.faces[0].attrs["texture"] = {"path": "/x/toro.png", "sw": 1, "sh": 1}
    toro.billboard = "mesh"
    assert is_scale_figure(mine)
    assert is_scale_figure(engineer_old)               # files of 26-09..15-10
    assert is_scale_figure(sumari_old)
    assert not is_scale_figure(placed_on_purpose)      # from the library tray
    assert not is_scale_figure(toro)


def test_preferences_choose_my_picture_and_ok_stores_it(native_host, tmp_path, monkeypatch):
    """The whole gesture: «My own picture…» asks for the file at once, OK
    copies it into the data folder and the next new document stands it."""
    from ingetrazo_render import shell_files
    from tests.preferences_chrome import Controls
    monkeypatch.setattr(sf, "figures_dir", lambda: tmp_path / "figures")
    src = _png(tmp_path / "Marco_Sumari.png")
    monkeypatch.setattr(shell_files.ShellFileDialog, "getOpenFileName",
                        staticmethod(lambda *a, **k: (str(src), "")))
    win = native_host.window
    ui = Controls(win, "general")
    assert not ui.visible("figure_image")
    ui.set("figure", sf.CUSTOM)
    assert ui.visible("figure_image")
    assert ui.value("figure_name") == "Marco Sumari"
    ui.set("figure_height", 1.68)
    fig = sf.chosen()
    assert fig.image.parent == tmp_path / "figures"
    assert (fig.height, fig.name) == (pytest.approx(1.68), "Marco Sumari")
    win._saved_version = win.viewport.scene.version
    win._on_new()
    assert [g.name for g in win.viewport.scene.groups
            if sf.is_marked(g)] == ["Marco Sumari"]


def test_cancelling_the_file_dialog_goes_back_to_the_person_before(native_host, monkeypatch):
    from ingetrazo_render import shell_files
    from tests.preferences_chrome import Controls
    monkeypatch.setattr(shell_files.ShellFileDialog, "getOpenFileName",
                        staticmethod(lambda *a, **k: ("", "")))
    win = native_host.window
    ui = Controls(win, "general")
    before = ui.value("figure")
    ui.set("figure", sf.CUSTOM)
    assert ui.value("figure") == before
