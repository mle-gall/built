# SPDX-License-Identifier: GPL-3.0-or-later
"""Saved scenes and sheet frames in the native Project browser."""
import pytest

from built_core.camera import OrbitCamera
from built_core.composition import Composicion, MarcoVista
from built_core.group import Group
from built_core.saved_views import SavedView
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_project_drawing_filters import Project
from views import wgpu_ui


@pytest.fixture
def project():
    p = Project()
    camera = OrbitCamera()
    p.scene.saved_views[:] = [SavedView.capture(n, p.scene, camera) for n in ("Plan L1", "North elevation")]
    plans = Composicion(name="A101 Plans", paper="A3")
    plans.frames = [MarcoVista(view_key="scene:Plan L1"), MarcoVista(view_key="scene:Section A", scale_n=50.)]
    elevations = Composicion(name="A102 Elevations")
    elevations.frames = [MarcoVista(view_key="scene:Plan L1", scale_n=200.), MarcoVista(view_key="std:front")]
    p.scene.compositions[:] = [plans, elevations]
    p.action("toggle", "saved-scenes:all")
    p.action("toggle", "sheet-pages:all")
    for page in (plans, elevations):
        p.action("toggle", key(p, page.name))
    return p


def rows(p):
    p.rows()
    return {text[1]: (text[0], text[2], state[0]) for text, state in p.view.renderer.calls["ui_set_tree"][0]}


def key(p, name):
    return rows(p)[name][0]


def test_the_sheets_tab_lists_scenes_sheets_and_their_views(project):
    r = rows(project)
    assert [name for name, data in r.items() if data[0].startswith("saved-scene:")] == ["Plan L1", "North elevation"]
    assert r["Plan L1"][1:] == ("On A101 Plans, A102 Elevations", 1)
    assert r["North elevation"][1:] == ("Not on a sheet", 1)
    assert r["A101 Plans"][1:] == ("A3", 1)
    assert r["Plan L1 · 1:100"][1:] == ("Scene", 2)
    assert r["Section A · 1:50"][1:] == ("Scene missing", 2)
    assert r["front · 1:100"][1:] == ("Standard view", 2)


def test_scene_usage_and_orphans_in_the_core(project):
    def detail(name):
        project.action("item", key(project, name))
        wgpu_ui._inspector(project.view, project.win)
        _, sections = project.view.renderer.calls["ui_set_inspector"]
        return dict(dict(sections)["Details"])
    assert detail("Plan L1")["Sheets"] == "A101 Plans, A102 Elevations"
    assert detail("Section A · 1:50") == {"Sheet": "A101 Plans", "Scale": "1:50"}
    r = rows(project)
    assert r["Section A · 1:50"][1] == "Scene missing"
    assert r["Plan L1 · 1:100"][1] == "Scene" and r["front · 1:100"][1] == "Standard view"


def test_deleting_a_scene_flags_its_frames_instead_of_breaking(project):
    p = project
    assert rows(p)["Plan L1 · 1:100"][1] == "Scene"
    del p.scene.saved_views[0]
    assert rows(p)["Plan L1 · 1:100"][1] == "Scene missing"
    assert p.scene.compositions[0].frames[0].view_key == "scene:Plan L1"


def test_a_sheet_added_in_the_composer_reaches_the_browser(project):
    p = project
    p.scene.compositions.append(Composicion(name="A103 Details"))
    original = key(p, "A103 Details")
    p.scene.compositions[-1].name = "A103 Joints"
    assert "A103 Details" not in rows(p)
    assert key(p, "A103 Joints") == original


def test_double_click_shows_a_scene_and_opens_a_sheet(project, native_host, monkeypatch):
    p = project
    w = native_host.window
    w.viewport.scene.compositions[:] = list(p.scene.compositions)
    w.viewport.scene.saved_views[:] = list(p.scene.saved_views)
    p.scene, p.win = w.viewport.scene, w
    group = Group(name="Tree")
    p.scene.groups.append(group)
    group.hidden = True
    p.scene.saved_views[0].hidden_objects = []
    p.action("open", key(p, "Plan L1"))
    assert w._scene_tabs.marked_view() is p.scene.saved_views[0]
    assert not group.hidden
    opened = []
    monkeypatch.setattr(w, "_show_sheet", opened.append)
    page, frame = key(p, "A102 Elevations"), key(p, "Section A · 1:50")
    p.action("open", page)
    p.action("open", frame)
    assert opened == [1, 0]
    p.scene.compositions.reverse()
    p.action("open", frame)
    assert opened == [1, 0, 1]
    p.scene.compositions.pop()
    p.action("open", frame)
    assert opened == [1, 0, 1]
