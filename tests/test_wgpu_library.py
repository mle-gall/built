# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Library dock place (views/wgpu_library.py): providers by type, the
rows the dock draws (sections, categories, sort, filter, folds), the dock
places (Project | Library, and back to where Project was), and the core
protocol commands the Library edits through."""
from __future__ import annotations

import pytest
from ingetrazo_core import Vec3 as V

from core import mesh as M
from core.group import Group
from core.history import History, InsertGroupCommand
from core.materials import Material
from core.protocol import commands as C
from core.scene import Scene
from views import wgpu_library as L


@pytest.fixture(autouse=True)
def _throwaway_settings(private_settings, monkeypatch):
    from PySide6.QtCore import QSettings
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


def test_one_materials_type_with_colour_and_image_bases():
    sc, _h = _painted_scene()
    items = L.MaterialsProvider()._model_items(_Ctx(sc))
    assert {e["name"]: e["base"] for e in items} == {"Brick": "image", "Steel grey": "colour", "Glass": "colour"}
    assert all(e["source"] == "model" for e in items)
    assert {e["name"]: e["uses"] for e in items} == {"Brick": 1, "Steel grey": 2, "Glass": 0}
    assert "delete" in items[0]["verbs"] and "apply" in items[0]["verbs"]
    # The bundled colours are a category of Materials, beside the images.
    bundled = L.MaterialsProvider()._bundled(_Ctx(sc))
    cats = {e["category"] for e in bundled}
    assert "Colours (RAL)" in cats and len(cats) > 1
    assert {e["base"] for e in bundled if e["category"] == "Colours (RAL)"} == {"colour"}


def test_facet_and_tokens_browse_by_base():
    items = _items()
    items[0]["base"], items[1]["base"] = "colour", "image"
    for e in items[2:]:
        e["base"] = "image"
    assert {e["key"] for e in L.filtered(items, _state(facet="colour"))} == {"m:b"}
    assert {e["key"] for e in L.filtered(items, _state(filter="is:image oak"))} == {"lib:w:0"}


def _items():
    def it(key, name, source, category="", uses=0, **kw):
        return dict({"key": key, "name": name, "source": source, "category": category, "uses": uses,
                     "hex": "#808080", "thumb": None, "verbs": ("apply",)}, **kw)
    return [it("m:b", "Beta", "model", uses=1), it("m:a", "Alpha", "model", uses=5),
            it("lib:w:0", "Oak", "bundled", "Wood"), it("lib:w:1", "Ash", "bundled", "Wood"),
            it("lib:s:0", "Slate", "bundled", "Stone"),
            it("lk:/p/Ride.igz|Rail", "Rail", "linked", file="/p/Ride.igz", uses=3)]


def _state(**kw):
    st = {"type": "materials", "view": 0, "sort": "name", "filter": "", "fold": set(), "open": set(),
          "recent": [], "mine_rev": 0, "scope": "s:model"}
    st.update(kw)
    return st


def _names(groups):
    return [(cap, [e["name"] for e in members]) for cap, _sub, members in groups]


def test_side_rows_scopes_then_categories_then_linked_files_read_only():
    rows = L.side_rows(_items(), _state())
    assert [(r[0], r[1], r[2], r[3]) for r in rows] == [
        ("s:model", "In this model", 2, 0), ("s:recent", "Recently used", 0, 0), ("s:mine", "My library", 0, 0),
        ("s:bundled", "Bundled", 3, 0), ("c:bundled:Stone", "Stone", 1, 1), ("c:bundled:Wood", "Wood", 2, 1),
        ("", "Linked files · read-only", 0, 0), ("s:linked:/p/Ride.igz", "Ride.igz", 1, 0)]
    assert rows[3][5] is True                           # Bundled folds, open
    assert rows[-1][4] == "file"
    folded = L.side_rows(_items(), _state(fold={"s:bundled"}))
    assert [r[0] for r in folded if r[0].startswith("c:")] == []


def test_a_scope_shows_its_items_sorted_and_a_search_shows_every_match():
    assert _names(L.groups_for(_items(), _state())) == [("", ["Alpha", "Beta"])]
    assert _names(L.groups_for(_items(), _state(sort="used"))) == [("", ["Alpha", "Beta"])]
    assert _names(L.groups_for(_items(), _state(sort="recent", recent=["m:b"]))) == [("", ["Beta", "Alpha"])]
    assert _names(L.groups_for(_items(), _state(scope="c:bundled:Wood"))) == [("", ["Ash", "Oak"])]
    assert _names(L.groups_for(_items(), _state(scope="s:bundled"))) == [("Stone", ["Slate"]), ("Wood", ["Ash", "Oak"])]
    assert _names(L.groups_for(_items(), _state(scope="s:linked:/p/Ride.igz"))) == [("", ["Rail"])]
    assert _names(L.groups_for(_items(), _state(scope="s:recent", recent=["lib:w:1", "m:a"]))) == [("", ["Ash", "Alpha"])]
    # A search looks everywhere, grouped by where it comes from.
    got = _names(L.groups_for(_items(), _state(filter="a", scope="c:bundled:Stone")))
    assert got == [("In this model", ["Alpha", "Beta"]), ("Bundled · Stone", ["Slate"]),
                   ("Bundled · Wood", ["Ash", "Oak"]), ("Ride.igz · read-only", ["Rail"])]


def test_providers_are_data_shaped_and_profiles_is_coming():
    ids = [p.id for p in L.PROVIDERS]
    assert ids[:3] == ["materials", "components", "profiles"]
    assert not L.provider("profiles").ready and L.provider("profiles").empty
    for p in L.PROVIDERS:
        for verb, label in p.verbs():
            assert isinstance(verb, str) and isinstance(label, str)


# ---- the sheet, the dock, the palette --------------------------------------------

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


def test_the_dock_has_no_library_place_any_more(monkeypatch):
    view, win = _View(), _Win()
    opened = []
    monkeypatch.setattr(L, "open_library", lambda w, t=None, **kw: opened.append(t) or True)
    L.set_mode(view, win, "problems")
    L.set_mode(view, win, "library")                    # the old switch, ⌘K, the strip: the sheet
    assert opened == [None]
    assert view._ui_lens == 2 and view._ui_mode == ""
    L.set_mode(view, win, "")
    assert view._ui_mode == ""


def test_placing_a_component_lands_on_components():
    view, win = _View(), _Win()
    win.viewport.active_tool = type("PlaceGroupTool", (), {})()
    L._state(view)["type"] = "materials"
    L._land(view, win)
    assert L._state(view)["type"] == "components"


def test_browsing_state_is_kept_between_sessions():
    view = _View()
    st = L._state(view)
    st["type"], st["view"], st["sort"], st["facet"] = "components", 1, "used", "image"
    L._save_state(st)
    again = _View()
    st2 = L._state(again)
    assert (st2["type"], st2["view"], st2["sort"], st2["facet"]) == ("components", 1, "used", "image")
    # A saved "colours" type (before Colours folded into Materials) opens Materials.
    st["type"] = "colours"
    L._save_state(st)
    assert L._state(_View())["type"] == "materials"


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
    from ingetrazo_core import Matrix4 as QMatrix4x4
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


# ---- one kind of resource: every paint is a named material -------------------------

def test_base_switch_keeps_pbr_values():
    sc, h = _painted_scene()
    out = C.run(h, "set_material_pbr", name="Steel grey", metallic=0.9, roughness=0.35,
                normal_map="/maps/steel_nrm.png")
    assert out["ok"]
    pbr = dict(sc.materials["Steel grey"].pbr)
    assert pbr == {"metallic": 0.9, "roughness": 0.35, "normal_map": {"path": "/maps/steel_nrm.png"}}
    post = sc.groups[0]
    assert post.mesh.faces[0].attrs["pbr"] == pbr
    out = C.run(h, "set_material_base", name="Steel grey", base="image",
                texture={"path": "/tex/brushed.png", "sw": 0.5, "sh": 0.5})
    assert out["ok"]
    mat = sc.materials["Steel grey"]
    assert mat.texture["path"] == "/tex/brushed.png" and mat.pbr == pbr
    assert post.mesh.faces[0].attrs["texture"]["path"] == "/tex/brushed.png"
    out = C.run(h, "set_material_base", name="Steel grey", base="color", color=[0.2, 0.2, 0.25])
    mat = sc.materials["Steel grey"]
    assert out["ok"] and mat.texture is None and mat.color == (0.2, 0.2, 0.25) and mat.pbr == pbr
    assert "texture" not in post.mesh.faces[0].attrs and post.mesh.faces[0].attrs["pbr"] == pbr
    # Removing every channel leaves no PBR at all (the glTF defaults would be fully metallic).
    C.run(h, "set_material_pbr", name="Steel grey", metallic=None, roughness=None, normal_map=None)
    assert sc.materials["Steel grey"].pbr is None
    with pytest.raises(ValueError):
        C.run(h, "set_material_pbr", name="Steel grey", shininess=1)
    with pytest.raises(ValueError):
        C.run(h, "set_material_base", name="Glass", base="image")


def test_create_material_reuses_the_same_recipe():
    sc, h = _painted_scene()
    out = C.run(h, "create_material", color=[0.4, 0.42, 0.45])
    assert out["reused"] and out["name"] == "Steel grey" and out["step"] is None
    out = C.run(h, "create_material", color=[1, 0, 0])
    assert out["ok"] and out["name"] == "Colour #FF0000" and "Colour #FF0000" in sc.materials
    out = C.run(h, "create_material", name="Oak", texture={"path": "/t/oak.png", "sw": 1, "sh": 1}, pbr={"roughness": 0.6})
    assert out["ok"] and sc.materials["Oak"].pbr == {"roughness": 0.6}
    with pytest.raises(ValueError):
        C.run(h, "create_material", color=[1, 0, 0], texture={"path": "/x.png"})


def test_raw_colour_paint_creates_or_reuses_a_named_material():
    from core.material_commands import material_for_paint
    sc, _h = _painted_scene()
    same = material_for_paint(sc, color=(0.4, 0.42, 0.45))
    assert same is sc.materials["Steel grey"]
    fresh = material_for_paint(sc, color=(1.0, 0.5, 0.0))
    assert fresh.name == "Colour #FF8000" and fresh.name not in sc.materials   # registered by the stroke
    image = material_for_paint(sc, texture={"path": "/t/red_brick.png", "sw": 1, "sh": 1, "uvw": [1, 2]})
    assert image.name == "red_brick" and "uvw" not in image.texture


def test_paint_tool_stamps_a_name_on_a_raw_colour(monkeypatch):
    """The Paint tool's click with a colour from the wheel: the face wears a
    material, registered in the same undo step."""
    from tools.paint import PaintTool
    sc, h = _painted_scene()
    face = sc.mesh.add_face([V(0, 5, 0), V(1, 5, 0), V(1, 6, 0)])
    monkeypatch.setattr(PaintTool, "current_is_default", False)
    monkeypatch.setattr(PaintTool, "current_texture", None)
    monkeypatch.setattr(PaintTool, "current_material", None)
    monkeypatch.setattr(PaintTool, "current_opacity", None)
    monkeypatch.setattr(PaintTool, "current_color", (0.0, 0.5, 1.0))
    from core.material_commands import material_for_paint
    from core.history import CompoundCommand, SetFaceColorCommand, SetFaceMaterialTagCommand
    # What on_click does before stamping (tools/paint.py, "No anonymous paint").
    mat = material_for_paint(sc, color=PaintTool.current_color)
    h.execute(CompoundCommand([SetFaceColorCommand([face], PaintTool.current_color),
                               SetFaceMaterialTagCommand([face], mat.name, mat)]))
    assert face.attrs["mat"] == "Colour #0080FF" and "Colour #0080FF" in sc.materials
    h.undo()
    assert "Colour #0080FF" not in sc.materials
    import inspect
    assert "material_for_paint" in inspect.getsource(PaintTool.on_click)


def test_unnamed_colours_and_name_these():
    from core.material_commands import unnamed_colours
    sc, h = _painted_scene()
    a = sc.mesh.add_face([V(0, 8, 0), V(1, 8, 0), V(1, 9, 0)])
    b = sc.mesh.add_face([V(3, 8, 0), V(4, 8, 0), V(4, 9, 0)])
    c = sc.mesh.add_face([V(6, 8, 0), V(7, 8, 0), V(7, 9, 0)])
    a.attrs["color"] = b.attrs["color"] = (1.0, 0.0, 0.0)          # older file: no "mat"
    c.attrs["color"] = (0.4, 0.42, 0.45)                           # Steel grey's colour
    c.attrs["back"] = {"color": (0.0, 1.0, 0.0)}
    found = unnamed_colours(sc)
    assert set(found) == {"#FF0000", "#666B73", "#00FF00"}
    assert len(found["#FF0000"]["faces"]) == 2
    rows = L.MaterialsProvider()._model_items(_Ctx(sc))
    assert {e["name"] for e in rows if e["category"] == "Unnamed colours"} == set(found)
    out = C.run(h, "name_unnamed_colours")
    assert out["ok"] and out["named"] == 3
    assert a.attrs["mat"] == b.attrs["mat"] == "Colour #FF0000"
    assert c.attrs["mat"] == "Steel grey"                         # reused, not duplicated
    assert c.attrs["back"]["mat"] == "Colour #00FF00"
    assert not unnamed_colours(sc)
    h.undo()
    assert "mat" not in a.attrs and "mat" not in c.attrs["back"]
    assert "Colour #FF0000" not in sc.materials and "Steel grey" in sc.materials
    assert len(unnamed_colours(sc)) == 3
    assert C.run(h, "name_unnamed_colours")["named"] == 3
