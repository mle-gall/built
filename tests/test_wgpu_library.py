# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Library dock place (views/wgpu_library.py): providers by type, the
rows the dock draws (sections, categories, sort, filter, folds), the dock
places (Project | Library, and back to where Project was), and the core
protocol commands the Library edits through."""
from __future__ import annotations

import pytest
from PySide6.QtGui import QVector3D as V

from core import mesh as M
from core.group import Group
from core.history import History, InsertGroupCommand
from core.materials import Material
from core.protocol import commands as C
from core.scene import Scene
from views import wgpu_library as L


@pytest.fixture(autouse=True)
def _throwaway_settings(tmp_path, monkeypatch):
    from PySide6.QtCore import QSettings
    QSettings.setDefaultFormat(QSettings.IniFormat)
    for scope in (QSettings.UserScope, QSettings.SystemScope):
        QSettings.setPath(QSettings.IniFormat, scope, str(tmp_path))
    QSettings().clear()
    monkeypatch.setenv("INGETRAZO_STORE", "0")


def _painted_scene():
    """Loose face in Brick (textured), a box group in Steel grey (plain),
    an unused Glass, and Steel grey on a container too."""
    sc = Scene()
    sc.materials["Brick"] = Material("Brick", texture={"path": "/nowhere/brick.png", "sw": 1.0, "sh": 0.6})
    sc.materials["Steel grey"] = Material("Steel grey", color=(0.4, 0.42, 0.45))
    sc.materials["Glass"] = Material("Glass", color=(0.6, 0.8, 0.9), opacity=0.3)
    f = sc.mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    f.attrs.update(sc.materials["Brick"].face_attrs())
    box = M.Mesh()
    g_face = box.add_face([V(5, 0, 0), V(6, 0, 0), V(6, 1, 0)])
    g_face.attrs.update(sc.materials["Steel grey"].face_attrs())
    g = Group(box, name="Post")
    other = Group(M.Mesh(), name="Plate")
    other.material = {"mat": "Steel grey", "color": (0.4, 0.42, 0.45)}
    h = History(sc)
    h.execute(InsertGroupCommand(g))
    h.execute(InsertGroupCommand(other))
    return sc, h


class _Ctx:
    def __init__(self, scene):
        self.scene = scene


def test_material_uses_count_objects_and_containers():
    sc, _h = _painted_scene()
    uses = L.material_uses(sc)
    assert uses["Brick"] == 1          # the loose mesh
    assert uses["Steel grey"] == 2     # the Post's mesh and the painted Plate
    assert uses["Glass"] == 0


def test_materials_and_colours_split_the_registry_by_image():
    sc, _h = _painted_scene()
    mats = [e for e in L.MaterialsProvider()._model_items(_Ctx(sc))]
    cols = [e for e in L.ColoursProvider()._model_items(_Ctx(sc))]
    assert [e["name"] for e in mats] == ["Brick"]
    assert sorted(e["name"] for e in cols) == ["Glass", "Steel grey"]
    assert all(e["source"] == "model" for e in mats + cols)
    assert {e["name"]: e["uses"] for e in cols} == {"Steel grey": 2, "Glass": 0}
    assert "delete" in cols[0]["verbs"] and "apply" in cols[0]["verbs"]


def _items():
    def it(key, name, source, category="", uses=0):
        return {"key": key, "name": name, "source": source, "category": category, "uses": uses,
                "hex": "#808080", "thumb": None, "verbs": ("apply",)}
    return [it("m:b", "Beta", "model", uses=1), it("m:a", "Alpha", "model", uses=5),
            it("lib:w:0", "Oak", "bundled", "Wood"), it("lib:w:1", "Ash", "bundled", "Wood"),
            it("lib:s:0", "Slate", "bundled", "Stone")]


def _state(**kw):
    st = {"type": "materials", "view": 0, "sort": "name", "filter": "", "fold": set(), "open": set(),
          "recent": [], "mine_rev": 0}
    st.update(kw)
    return st


def _shown(rows):
    return [(r[1][0], r[0][1]) for r in rows]


def test_rows_sections_in_order_bundled_categories_folded():
    rows, _ = L.rows_for(_items(), _state())
    assert _shown(rows) == [(0, "In this model"), (2, "Alpha"), (2, "Beta"),
                            (0, "My library"), (3, rows[4][0][1]),
                            (0, "Bundled"), (1, "Stone"), (1, "Wood")]
    # Counts everywhere: a section counts its items, a category its own.
    counts = {r[0][1]: r[1][1] for r in rows if r[1][0] in (0, 1)}
    assert counts == {"In this model": 2, "My library": 0, "Bundled": 3, "Stone": 1, "Wood": 2}


def test_rows_open_category_sort_and_filter():
    rows, _ = L.rows_for(_items(), _state(open={"c:bundled:Wood"}))
    assert (2, "Ash") in _shown(rows) and (2, "Oak") in _shown(rows)
    rows, _ = L.rows_for(_items(), _state(sort="used"))
    model = [n for k, n in _shown(rows) if k == 2][:2]
    assert model == ["Alpha", "Beta"]
    rows, _ = L.rows_for(_items(), _state(sort="recent", recent=["m:b"]))
    assert [n for k, n in _shown(rows) if k == 2][:2] == ["Beta", "Alpha"]
    # A filter opens what matches (categories too) and drops empty sections.
    rows, by_key = L.rows_for(_items(), _state(filter="wood"))
    assert _shown(rows) == [(0, "Bundled"), (1, "Wood"), (2, "Ash"), (2, "Oak")]
    assert set(by_key) == {"lib:w:0", "lib:w:1"}


def test_folded_section_hides_its_items_and_marks_the_current_paint():
    rows, _ = L.rows_for(_items(), _state(fold={"s:model"}), current="Alpha")
    assert (2, "Alpha") not in _shown(rows)
    rows, _ = L.rows_for(_items(), _state(), current="Alpha")
    alpha = next(r for r in rows if r[0][1] == "Alpha")
    assert alpha[1][2] == 1 and alpha[1][1] == 5      # active, uses


def test_providers_are_data_shaped_and_profiles_is_coming():
    ids = [p.id for p in L.PROVIDERS]
    assert ids[:4] == ["materials", "colours", "components", "profiles"]
    assert not L.provider("profiles").ready and L.provider("profiles").empty
    for p in L.PROVIDERS:
        for verb, label in p.verbs():
            assert isinstance(verb, str) and isinstance(label, str)


# ---- dock places ----------------------------------------------------------------

class _Renderer:
    def __init__(self):
        self.calls = []

    def __getattr__(self, name):
        return lambda *a: self.calls.append((name, a))


class _View:
    def __init__(self):
        self.renderer = _Renderer()

    def update(self):
        pass


class _Win:
    def __init__(self):
        self.viewport = type("VP", (), {"active_tool": None})()
        self._tool_actions = {}


def test_library_returns_project_to_where_it_was():
    view, win = _View(), _Win()
    L.set_mode(view, win, "problems")
    L.set_mode(view, win, "library")
    assert view._ui_mode == "library"
    assert ("ui_library_opened", ()) in view.renderer.calls
    # The Project segment, or Esc: back to Problems, not to the bare tree.
    L.set_mode(view, win, "")
    assert view._ui_mode == "problems"
    # From Problems, Esc (or ×) is the tree.
    L.set_mode(view, win, "")
    assert view._ui_mode == ""
    L.set_mode(view, win, "library")
    L.set_mode(view, win, "")
    assert view._ui_mode == ""


def test_placing_a_component_lands_on_components():
    view, win = _View(), _Win()
    win.viewport.active_tool = type("PlaceGroupTool", (), {})()
    L._state(view)["type"] = "colours"
    L.set_mode(view, win, "library")
    assert L._state(view)["type"] == "components"


def test_browsing_state_is_kept_between_sessions():
    view = _View()
    st = L._state(view)
    st["type"], st["view"], st["sort"] = "colours", 1, "used"
    L._save_state(st)
    again = _View()
    st2 = L._state(again)
    assert (st2["type"], st2["view"], st2["sort"]) == ("colours", 1, "used")


# ---- protocol commands -----------------------------------------------------------

def test_rename_duplicate_edit_delete_material_through_the_protocol():
    sc, h = _painted_scene()
    out = C.run(h, "rename_material", name="Steel grey", new_name="Anthracite")
    assert out["ok"] and "Anthracite" in sc.materials and "Steel grey" not in sc.materials
    post = sc.groups[0]
    assert post.mesh.faces[0].attrs["mat"] == "Anthracite"
    assert sc.groups[1].material["mat"] == "Anthracite"
    with pytest.raises(ValueError):
        C.run(h, "rename_material", name="Anthracite", new_name="Brick")
    out = C.run(h, "duplicate_material", name="Anthracite")
    assert out["ok"] and out["name"] == "Anthracite copy" and "Anthracite copy" in sc.materials
    assert C.run(h, "duplicate_material", name="Anthracite")["name"] == "Anthracite copy 2"
    out = C.run(h, "edit_material", name="Anthracite", color=[1, 0, 0])
    assert out["ok"] and post.mesh.faces[0].attrs["color"] == (1.0, 0.0, 0.0)
    out = C.run(h, "edit_material", name="Anthracite", opacity=0.5)
    assert out["ok"] and post.mesh.faces[0].attrs["opacity"] == 0.5
    with pytest.raises(ValueError):
        C.run(h, "edit_material", name="Anthracite", tile=[1, 1])
    out = C.run(h, "delete_material", name="Anthracite")
    assert out["ok"] and "Anthracite" not in sc.materials
    assert "mat" not in post.mesh.faces[0].attrs and "color" not in post.mesh.faces[0].attrs
    assert sc.groups[1].material is None
    h.undo()
    assert "Anthracite" in sc.materials and post.mesh.faces[0].attrs["mat"] == "Anthracite"
    assert sc.groups[1].material["mat"] == "Anthracite"
    with pytest.raises(KeyError):
        C.run(h, "delete_material", name="nope")


def test_delete_placements_and_rename_component():
    sc, h = _painted_scene()
    proto = M.Mesh()
    proto.add_face([V(9, 0, 0), V(10, 0, 0), V(10, 1, 0)])
    a, b = Group(proto, name="Chair"), Group(proto, name="Chair")
    from PySide6.QtGui import QMatrix4x4
    for g in (a, b):
        g.xform, g.component = QMatrix4x4(), True
    h.execute(InsertGroupCommand(a))
    h.execute(InsertGroupCommand(b))
    out = C.run(h, "rename_component", placement=a.uid, name="Stool")
    assert out["ok"] and a.name == b.name == "Stool"
    out = C.run(h, "delete_placements", placements=[a.uid, b.uid])
    assert out["ok"] and a not in sc.groups and b not in sc.groups
    h.undo()
    assert a in sc.groups and b in sc.groups
    with pytest.raises(KeyError):
        C.run(h, "delete_placements", placements=["nope"])
