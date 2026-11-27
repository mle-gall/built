# SPDX-License-Identifier: GPL-3.0-or-later
"""The Library on the large sheet (built_render.wgpu_library, option B with C's
palette): types as tabs, scopes and categories on the side with the linked
files' materials read-only, 96-pt tiles, the details pane and its editor;
the Paint strip's fast path and Browse all…; the dock's inspector editing
the material Paint holds. Offscreen: a stand-in renderer records the
sheet's JSON."""
from __future__ import annotations

from tests.raster_samples import solid_raster
import json

import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from built_core import Vec3 as V

from built_core.materials import Material
from built_render import chrome_sheet as cs
from built_render import wgpu_library as L


class Renderer:
    def __init__(self):
        self.calls = []

    def ui_set_large_sheet(self, text):
        cs.check_wire(text)       # what the chrome reads (ui/large_sheet.rs)
        self.calls.append(("sheet", json.loads(text) if text else None))

    def __getattr__(self, name):
        return lambda *a: self.calls.append((name, a))

    def sheet(self):
        return [c[1] for c in self.calls if c[0] == "sheet"][-1]

    def last(self, name):
        return [c[1] for c in self.calls if c[0] == name][-1]


@pytest.fixture
def win(native_host, monkeypatch):
    w = native_host.window
    view = native_host.view
    monkeypatch.setattr(view, "renderer", Renderer())
    monkeypatch.setattr(view, "_ui_attached", True, raising=False)
    sc = w.viewport.scene
    sc.materials["Beech"] = Material("Beech", color=(0.75, 0.55, 0.35))
    sc.materials["Concrete"] = Material("Concrete", color=(0.6, 0.6, 0.6))
    f = sc.mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    f.attrs.update(sc.materials["Beech"].face_attrs())
    sc.version += 1
    return w


def frame(win):
    cs.before_frame(win.view, win)
    job = getattr(win.viewport.scene, "_library_usage_job", None)
    if L._state(win.view)["type"] == "materials":
        # Flushing tracked edits can supersede the first worker's document stamp.
        for _ in range(3):
            if not job:
                break
            job[1].result(timeout=5)
            cs.before_frame(win.view, win)
            current = win.viewport.scene._library_usage_job
            if current is job:
                break
            job = current
        else:
            pytest.fail("The material index kept invalidating without edits")
    return win.view.renderer.sheet()


def act(win, verb, *args):
    cs.action(win, cs.SEP.join(("library", verb) + args))


def tiles(sh):
    return [i for b in sh["content"]["blocks"] if b["kind"] in ("rows", "tiles") for i in b["items"]]


def detail(sh):
    return sh["detail"]["blocks"]


def test_browse_all_opens_the_sheet_on_the_models_materials(win):
    assert L.open_library(win, "materials")
    sh = frame(win)
    assert sh["id"] == "library"
    assert [(t["id"], t["on"], t["enabled"]) for t in sh["tabs"]] == [
        ("materials", True, True), ("components", False, True), ("families", False, True)]
    assert sh["search"]["placeholder"] == "Search all materials"
    side = [(s["id"], s["label"]) for s in sh["side"]]
    assert side[:4] == [("s:model", "In this model"), ("s:recent", "Recently used"), ("s:mine", "My library"),
                        ("s:bundled", "Bundled")]
    assert any(i.startswith("c:bundled:") for i, _ in side)          # its categories
    assert sh["content"]["title"] == "In this model"
    t = tiles(sh)
    assert [x["label"] for x in t] == ["Beech", "Concrete"]
    assert t[0]["sub"] == "colour · used by 1" and t[0]["hex"]
    assert next(b for b in sh["content"]["blocks"] if b["kind"] == "rows")["tone"] == "thumbs"
    assert sh["focus"] == "m:Beech"                                   # its details on the right
    heading = next(b for b in detail(sh) if b["kind"] == "heading")
    assert heading["text"] == "Beech" and heading["right"] == "In this model"
    # Purge and Name these: under the model's tiles.
    acts = next(b for b in sh["content"]["blocks"] if b["kind"] == "buttons")["buttons"]
    assert acts[-1]["id"] == "typeact:purge"


def test_scopes_categories_search_and_sort(win):
    L.open_library(win)
    frame(win)
    act(win, "side", "c:bundled:Colours (RAL)")
    sh = frame(win)
    assert sh["content"]["title"] == "Colours (RAL)" and sh["content"]["sub"].startswith("Bundled · ")
    ral = tiles(sh)
    assert len(ral) > 100 and all(x["hex"] for x in ral) and ral[0]["label"].startswith("RAL ")
    heading = next(b for b in detail(sh) if b["kind"] == "heading")
    assert heading["right"] == "Bundled · read-only"
    act(win, "search", "beech")
    sh = frame(win)
    caps = [b["text"] for b in sh["content"]["blocks"] if b["kind"] == "caption"]
    assert caps[0] == "In this model"                                 # every match, by where it is
    assert not any(s.get("on") for s in sh["side"])                   # no side row while searching
    act(win, "search", "")
    act(win, "side", "s:model")
    assert [m["id"] for m in frame(win)["menus"]][:2] == ["sort", "show"]
    act(win, "headmenu", "show", "facet:image")
    assert tiles(frame(win)) == []                                    # no image bases in this model
    act(win, "headmenu", "show", "facet:all")
    act(win, "headmenu", "sort", "used")
    assert L._state(win.view)["sort"] == "used"
    assert frame(win)["menus"][0]["label"] == "Sort: Most used"


def test_the_editor_applies_edits_and_sliders_when_they_settle(win, monkeypatch):
    L.open_library(win)
    assert not any(b["kind"] == "fields" for b in detail(frame(win)))
    act(win, "verb", "edit")
    view = win.view
    assert not cs.shown(win) and L.inspector_key(view, win) == "m:Beech"
    sc = win.viewport.scene
    title, sections = L.inspector(view, win)
    assert title == "Beech" and {"Name", "Colour", "Base"} <= dict(sections[0][1]).keys()
    L.edit_field(win, view, "color:Beech", "#336699")
    assert L._hex(sc.materials["Beech"].color) == "#336699"
    L.edit_field(win, view, "pbr:Beech", "On")
    assert sc.materials["Beech"].pbr == {"metallic": 0., "roughness": .5}
    rows = dict(L.inspector(view,win)[1])["Finish"]
    assert "slider" in dict(rows)["Roughness"]
    before = win.viewport.history.revision
    L.edit_field(win, view, "percent_roughness:Beech", "62")
    assert win.viewport.history.revision == before + 1
    assert sc.materials["Beech"].pbr["roughness"] == pytest.approx(.62)
    picked = []
    monkeypatch.setattr(L, "_pick_image", lambda w, title: picked.append(title))
    L.edit_field(win, view, "base:Beech", "Image")
    L.action(win, view, "map:m:Beech:roughness_map")
    assert picked == ["Base image", "Roughness map"]
    L.edit_field(win, view, "pbr:Beech", "Off")
    assert sc.materials["Beech"].pbr is None


def test_the_editor_sets_emission_colour_strength_and_map(win, monkeypatch):
    L.open_library(win)
    frame(win)
    act(win, "verb", "edit")
    view, sc = win.view, win.viewport.scene
    look = dict(dict(L.inspector(view, win)[1])["Finish"])
    assert look["Emission"].endswith("#000000") and "Strength" in look
    before = win.viewport.history.revision
    L.edit_field(win, view, "emission:Beech", "#ff8000")
    assert win.viewport.history.revision == before + 1                # one undo step
    pbr = sc.materials["Beech"].pbr
    assert pbr["emission"] == pytest.approx([1, 128 / 255, 0], abs=.01) and pbr["emission_strength"] == 1.0
    L.edit_field(win, view, "emission_strength:Beech", "3,5")
    assert sc.materials["Beech"].pbr["emission_strength"] == 3.5
    picked = []
    monkeypatch.setattr(L, "_pick_image", lambda w, title: picked.append(title) or "/x/glow.png")
    L.action(win, view, "map:m:Beech:emission_map")
    assert picked == ["Emission map"] and sc.materials["Beech"].pbr["emission_map"] == {"path": "/x/glow.png"}
    look = dict(dict(L.inspector(view, win)[1])["Finish"])
    assert look["Emission map"] == "glow.png"
    win.viewport.history.undo()
    win.viewport.history.undo()
    win.viewport.history.undo()
    assert "emission" not in (sc.materials["Beech"].pbr or {})


def test_paint_with_it_closes_the_sheet_and_remembers_it(win, monkeypatch):
    painted = []
    monkeypatch.setattr("views.wgpu_ui._paint_with", lambda w, key: painted.append(key))
    L.open_library(win)
    frame(win)
    act(win, "side", "c:bundled:Colours (RAL)")
    sh = frame(win)
    key = tiles(sh)[0]["id"]
    act(win, "open", key)                                             # double-click or ↩
    assert painted == [key]
    assert not cs.shown(win)
    assert L._state(win.view)["recent"][0] == key
    # Recently used lists it, and the strip offers it next to the model's.
    L.open_library(win)
    frame(win)
    act(win, "side", "s:recent")
    assert [x["id"] for x in tiles(frame(win))] == [key]


def test_delete_asks_first_over_the_sheet(win, monkeypatch):
    asked = []
    monkeypatch.setattr("built_render.chrome_dialogs.ask_later",
                        lambda parent, title, body, buttons, done, **kw: (asked.append(title), done("yes")))
    L.open_library(win)
    frame(win)
    act(win, "focus", "m:Concrete")
    act(win, "verb", "delete")
    assert asked == ["Delete “Concrete”?"]
    assert "Concrete" not in win.viewport.scene.materials
    assert cs.shown(win)                                              # still browsing


def test_linked_files_materials_are_listed_read_only(win, tmp_path):
    from tests import composite_fixture as cf
    from built_core import links
    from built_core.materials import Material as M
    from formats import igz
    files = cf.build(tmp_path / "comp", big=12)
    # Paint the source's parts, then link it into the open document.
    from built_core.scene import Scene
    src = Scene()
    igz.load_into(src, files["c"])
    src.materials["Galvanised"] = M("Galvanised", color=(0.7, 0.72, 0.74))
    for g in src.groups:
        for f in g.mesh.faces:
            f.attrs.update(src.materials["Galvanised"].face_attrs())
    igz.save_scene(src, files["c"])
    sc = win.viewport.scene
    comp = links.make_link(sc, files["c"])
    sc.groups.append(comp)
    sc.version += 1
    L.open_library(win)
    sh = frame(win)
    side = [(s["id"], s["label"], s.get("kind", "")) for s in sh["side"]]
    assert ("", "Linked files · read-only", "caption") in side
    lid = next(i for i, lab, k in side if lab == "C.igz")
    assert lid.startswith("s:linked:")
    assert "Galvanised" not in [x["label"] for x in tiles(sh)]       # not the host's own
    act(win, "side", lid)
    sh = frame(win)
    assert [x["label"] for x in tiles(sh)] == ["Galvanised"]
    act(win, "focus", tiles(sh)[0]["id"])
    sh = frame(win)
    assert next(b for b in detail(sh) if b["kind"] == "heading")["right"] == "C.igz · read-only"
    assert not any(b["kind"] == "fields" for b in detail(sh))         # no editor: read-only
    verbs = [v["id"] for b in detail(sh) if b["kind"] == "buttons" for v in b["buttons"]]
    assert verbs == ["apply", "opensource"]


def test_components_tab(win):
    L.open_library(win, "components")
    sh = frame(win)
    assert sh["tabs"][1]["on"] and sh["search"]["placeholder"] == "Search all components"
    act(win, "search", "source:bundled")
    sh = frame(win)
    assert {r["label"] for r in sh["side"]} >= {"People (2D)", "Starters (3D)"}
    assert tiles(sh) and all(row["component_source"] == "Bundled" for row in tiles(sh))
    verbs = [v["id"] for b in detail(sh) if b["kind"] == "buttons" for v in b["buttons"]]
    assert verbs[0] == "apply" and next(v for b in detail(sh) if b["kind"] == "buttons"
                                        for v in b["buttons"])["label"] == "Place a copy"


# ---- C's palette and the inspector ------------------------------------------------

def _paint_on(win, monkeypatch, name="Beech"):
    from tools.paint import PaintTool
    monkeypatch.setattr("views.wgpu_ui._active_tool_id", lambda w: "paint")
    monkeypatch.setattr(PaintTool, "current_material", win.viewport.scene.materials.get(name))


def test_the_strip_paints_from_the_model_and_recent_and_browses_all(win, monkeypatch):
    _paint_on(win, monkeypatch)
    view = win.view
    L._remember(view, "ral:RAL 7016")
    L.before_frame(view, win)
    items = view.renderer.last("ui_set_palette")[0]
    ids = [i[0] for i in items]
    assert ids[:2] == ["m:Beech", "m:Concrete"]                       # most used first
    assert items[0][4] is True                                        # the current one
    assert ids[2] == "-" and ids[3] == "ral:RAL 7016"                 # then the recent ones
    assert items[-1][0] == "" and items[-1][1] == "Beech"             # its name, after a rule
    painted = []
    monkeypatch.setattr("views.wgpu_ui._paint_with", lambda w, key: painted.append(key))
    L.paint_from_strip(win, "m:Concrete")
    assert painted == ["m:Concrete"] and L._state(view)["recent"][0] == "m:Concrete"
    # Browse all… (the strip's button): the chrome sends libsheet:materials.
    from views import wgpu_ui
    wgpu_ui._run(win, ["libsheet:materials"])
    assert cs.current(win) is L._sheet_provider(view)
    # Not painting: no strip.
    monkeypatch.setattr("views.wgpu_ui._active_tool_id", lambda w: "select")
    L.before_frame(view, win)
    assert view.renderer.last("ui_set_palette")[0] == []


def test_the_inspector_edits_the_material_paint_holds(win, monkeypatch):
    view = win.view
    assert L.inspector_key(view, win) is None
    _paint_on(win, monkeypatch, "Concrete")
    assert L.inspector_key(view, win) == "m:Concrete"
    title, sections = L.inspector(view, win)
    assert title == "Concrete"
    rows = dict(sections[0][1])
    assert "Name" in rows and "Base" in rows
    chips = [a for t, r in sections if t == "Actions" for a, _l in r]
    assert "lib:browse:materials" in chips
    L.edit_field(win, view, "opacity:Concrete", "50")
    assert win.viewport.scene.materials["Concrete"].opacity == pytest.approx(0.5)


def test_online_catalogue_search_preview_download_and_document_credit(win, tmp_path, monkeypatch):
    from built_core import component_library as library
    from formats import igz
    from built_core.scene import Scene
    import zipfile
    import time

    src = tmp_path / "source"
    (src / "miniaturas").mkdir(parents=True)
    (src / "modelos").mkdir()
    entries = [{"id": "stool", "nombre": "Taburete", "nombre_en": "Stool",
                "categoria": "Cocina", "autor": "Ana", "licencia": "CC BY", "obj": "stool.obj"},
               {"id": "chair", "nombre": "Silla", "nombre_en": "Chair", "categoria": "Oficina"}]
    (src / "index.json").write_text(json.dumps({"modelos": entries}))
    with zipfile.ZipFile(src / "modelos/stool.zip", "w") as z:
        z.writestr("stool.obj", "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")
        z.writestr("LICENSE.txt", "CC BY · Ana")
    image = solid_raster(32, 32, (69, 50, 171))
    image.save(str(src / "miniaturas/stool.png"))
    monkeypatch.setenv("BUILT_LIBRARY", str(src))
    monkeypatch.setenv("BUILT_TEXTURE_CACHE", str(tmp_path / "cache/tex"))

    assert L.open_library(win, "components", scope="s:online")
    assert frame(win)["content"]["title"] == "Component library"
    session = L._sheet_provider(win.view).catalogue

    def finish():
        session.job.result(timeout=5)
        session.poll()
        return frame(win)

    loaded = finish()
    assert [i["label"] for i in tiles(loaded)] == ["Chair", "Stool"]
    assert not library.cached_thumbnail("stool")
    act(win, "side", "online-cat:Oficina")
    assert [i["label"] for i in tiles(frame(win))] == ["Chair"]
    for query in ("taburete", "stool"):
        act(win, "search", query)
        shown = frame(win)
        assert [i["label"] for i in tiles(shown) if i["id"].startswith("online:")] == ["Stool"]
    act(win, "focus", "online:stool")
    assert any("Stool · CC BY · by Ana" in b.get("text", "") for b in detail(frame(win)))
    act(win, "visible", "online:stool", "online:bogus")
    deadline = time.monotonic() + 3
    while library.thumbnails_pending() and time.monotonic() < deadline:
        time.sleep(.01)
    session.poll()
    frame(win)
    icons = win.view.renderer.last("ui_set_icons")[0]
    assert any(key == "online:stool" and w == h > 0 for key, w, h, _ in icons)
    assert not library.cached_thumbnail("chair")
    assert not (library.library_cache() / "modelos").exists()
    placed = []
    monkeypatch.setattr(win, "_start_place", placed.append)
    act(win, "verb", "insert_online")
    finish()
    assert not cs.shown(win) and len(placed) == 1
    assert (library.library_cache() / "modelos/stool/LICENSE.txt").read_text() == "CC BY · Ana"
    win.viewport.scene.groups.append(placed[0])
    win.viewport.scene.version += 1
    path = tmp_path / "credit.built"
    igz.save_scene(win.viewport.scene, path)
    restored = Scene()
    igz.load_into(restored, path)
    assert restored.groups[-1].ext["catalogue"]["autor"] == "Ana"
    assert restored.groups[-1].ext["catalogue"]["licencia"] == "CC BY"
    L.open_library(win, "components", scope="s:model")
    candidates = tiles(frame(win))
    saved = next(it["id"] for it in candidates if it["label"] == placed[0].name)
    act(win, "focus", saved)
    assert any("Stool · CC BY · by Ana" in b.get("text", "") for b in detail(frame(win)))


def test_failed_or_abandoned_catalogue_download_never_places(win, monkeypatch):
    from built_core import component_library as library
    from threading import Event

    monkeypatch.setenv("BUILT_LIBRARY", "https://example.invalid/catalogue")
    entries = [{"id":"chair", "nombre":"Silla", "nombre_en":"Chair"}]
    monkeypatch.setattr(library, "index", lambda refresh: entries)
    monkeypatch.setattr(library, "model_file", lambda row: None)
    placed = []
    monkeypatch.setattr(win, "insert_library_component", lambda *args: placed.append(args))
    assert L.open_library(win, "components", scope="s:online")
    frame(win)
    session = L._sheet_provider(win.view).catalogue
    session.job.result(timeout=3)
    session.poll()
    frame(win)
    act(win, "open", "online:chair")
    session.job.result(timeout=3)
    session.poll()
    assert any("Could not download" in b.get("text", "") for b in detail(frame(win)))
    gate = Event()
    monkeypatch.setattr(library, "model_file", lambda row: gate.wait(3))
    act(win, "open", "online:chair")
    act(win, "side", "s:model")
    gate.set()
    win._host.idle()
    assert not placed


def test_family_collection_import_reopen_and_partial_report(win, tmp_path, monkeypatch):
    import json
    import sys
    import time
    from types import SimpleNamespace
    from tests.test_rfa_import import payload

    monkeypatch.setenv("BUILT_SETTINGS_DIR", str(tmp_path / "prefs"))
    folder = tmp_path / "Sample set"
    folder.mkdir()
    for name in ("cabinet.RFA", "invalid.rfa"):
        (folder / name).touch()

    def read(path):
        from pathlib import Path
        if Path(path).name == "invalid.rfa":
            raise ValueError("Invalid family")
        return json.dumps(payload())

    monkeypatch.setitem(sys.modules, "built_rfa", SimpleNamespace(read_family=read))
    (folder / "cabinet.txt").write_text(",Width##LENGTH##MILLIMETERS\nWide,1600\n")
    L.import_collection(folder)
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        paths = list(tmp_path.glob("prefs/data/Built/collections/*/collection.json"))
        if paths:
            break
        time.sleep(.025)
    assert paths
    manifest = json.loads(paths[0].read_text())
    assert manifest["name"] == "Sample set"
    assert any("invalid.rfa" in s for s in manifest["reports"])
    assert [r["name"] for r in manifest["items"]] == ["Standard", "Empty", "Wide"]
    assert L.open_library(win, "families")
    sh = frame(win)
    act(win, "side", "s:" + manifest["source"])
    assert {r["label"] for r in tiles(frame(win))} == {"Standard", "Empty", "Wide"}


@pytest.mark.parametrize(("category", "class_name"), [("Doors", "IfcDoor"), ("Windows", "IfcWindow")])
def test_family_placement_keeps_type_and_records_wall_in_one_undo(win, tmp_path, monkeypatch, category, class_name):
    from built_core import Matrix4
    from built_core.group import Group
    from tests.test_ifc_vocabulary_relationships import mesh_at
    from tests.test_rfa_import import payload
    from core.bim_registry import get
    from built_core.command_protocol import run

    data = payload()
    data["family"]["category"] = category
    path = tmp_path / "family.json"
    path.write_text(json.dumps(data))
    row = {"file": path, "index": 1, "name": "Empty", "family_category": category, "key": "family:test"}
    armed = []
    monkeypatch.setattr(win, "_start_place", lambda g, align: armed.append((g, align)))
    ctx = L.Ctx(win, win.view)
    L.provider("families").run(ctx, "apply", row)
    group, align = armed.pop()
    assert align and group.ifc["class"] == class_name and group.name == "Empty"
    history = win.viewport.history
    scene = win.viewport.scene
    group.ifc["psets"][".rfa parameters"]["Opening Width"] = {"kind": "length", "value": .8}
    wall = Group(mesh_at(0), "Wall")
    wall.xform = Matrix4()
    wall.xform.scale(1, 1, 2)
    wall.ifc = {"class": "IfcWall"}
    scene.groups.append(wall)
    previous = list(scene.groups)
    result = run(history, "place_group", template=group.uid, _pending=group,
                 basis=[[1, 0, 0], [0, 1, 0], [0, 0, 1]], offset=[.2, -.6, .1],
                 wall_host={"uid": wall.uid, "sill": .1, "offset": 0, "flipped": False})
    assert result["ok"] and scene.groups[-1] is group
    assert group.ext["wall_host"]["uid"] == wall.uid
    left, _, right, _ = wall.ifc["openings"][0]["wall_rect"]
    assert 2 * (right - left) == pytest.approx(.8)
    assert get(scene)["types"][group.ifc["type"]]["ifc_class"] == class_name
    history.undo()
    assert list(scene.groups) == previous and "family:test" not in get(scene)["types"]


def test_hosted_window_through_cut_history_and_ifc(win, tmp_path):
    from built_core import Matrix4, Vec3
    from built_core.group import Group
    from built_core.command_protocol import run
    from core.bim import face_set_volume
    from formats.ifc import save_ifc
    from tests.test_geometry_arrays import CUBE
    pytest.importorskip("ifcopenshell")
    import ifcopenshell.geom
    import numpy as np

    scene, history = win.viewport.scene, win.viewport.history
    wall = Group(name="Wall")
    for loop in CUBE:
        wall.mesh.add_face([Vec3(8*x, .25*y, 3*z) for x, y, z in loop])
    wall.ifc = {"class": "IfcWall"}
    wall.xform = Matrix4()
    sibling = Group(wall.mesh, "Another wall")
    sibling.xform = Matrix4()
    sibling.xform.translate(0, 5, 0)
    scene.groups.extend([wall, sibling])
    original = wall.mesh

    def place(label, x, sill):
        g = Group(name=label)
        g.ifc = {"class": "Ifc" + label}
        g.ext = {"library_host": "wall"}
        g.xform = Matrix4()
        for loop in CUBE:
            g.mesh.add_face([Vec3(u, .12*v, z) for u, v, z in loop])
        ring = [[x, 0, sill], [x+1, 0, sill], [x+1, 0, sill+1], [x, 0, sill+1]]
        result = run(history, "place_group", template=g.uid, _pending=g,
                     basis=[[1,0,0],[0,1,0],[0,0,1]], offset=[x,0,sill],
                     wall_host=dict(uid=wall.uid, outline=ring, normal=[0,-1,0],
                                    initial_offset=0, offset=0, sill=sill, flipped=False))
        assert result["ok"], result
        return g

    window = place("Window", 2, 1)
    assert wall.mesh is not original and sibling.mesh is original
    assert sum(len(f.holes) for f in wall.mesh.faces) == 2
    assert len(wall.mesh.faces) == 10
    assert face_set_volume(wall.mesh.faces) == pytest.approx(5.75)
    history.undo()
    assert wall.mesh is original and window not in scene.groups
    history.redo()
    door = place("Door", 5, 0)
    assert face_set_volume(wall.mesh.faces) == pytest.approx(5.5)
    for dx in (3.1, -2.5):
        mesh = wall.mesh
        assert not run(history, "move_placements", placements=[window.uid], delta=[dx,0,0])["ok"]
        assert wall.mesh is mesh and window.xform.data()[12] == pytest.approx(2)
    assert run(history, "move_placements", placements=[window.uid], delta=[1,.03,.2])["ok"]
    assert window.ext["wall_host"]["offset"] == pytest.approx(.03)
    assert window.ext["wall_host"]["sill"] == pytest.approx(1.2)
    assert run(history, "set_opening_position", placement=window.uid, sill=1.4, offset=.07)["ok"]
    assert window.ext["wall_host"]["sill"] == pytest.approx(1.4)
    history.undo()
    assert window.ext["wall_host"]["offset"] == pytest.approx(.03)
    assert run(history, "set_group_scale", placement=window.uid, factors=[1.2,1,1.1])["ok"]
    assert face_set_volume(wall.mesh.faces) == pytest.approx(6 - .25*(1.2*1.1 + 1))
    history.undo()
    assert run(history, "flip_selection", placements=[window.uid], center=[3,.03,1.2], axis=[0,1,0], copy=False)["ok"]
    assert window.ext["wall_host"]["flipped"] and len(wall.ifc["openings"]) == 2
    history.undo()
    path = tmp_path / "openings.ifc"
    assert not save_ifc(scene, path).problems
    model = ifcopenshell.open(str(path))
    assert [len(model.by_type(c)) for c in ("IfcOpeningElement", "IfcRelVoidsElement", "IfcRelFillsElement")] == [2,2,2]
    settings = ifcopenshell.geom.settings()
    for opening in model.by_type("IfcOpeningElement"):
        assert opening.VoidsElements[0].RelatingBuildingElement.is_a("IfcWall")
        filling = opening.HasFillings[0].RelatedBuildingElement
        solid = ifcopenshell.geom.create_shape(settings, opening)
        xyz = np.array(solid.geometry.verts).reshape(-1, 3)
        expected = [3,0,1.2] if filling.is_a("IfcWindow") else [5,0,0]
        assert xyz.min(axis=0) == pytest.approx(expected)
        assert np.ptp(xyz, axis=0) == pytest.approx([1,.25,1])
    history.undo()
    assert window.ext["wall_host"]["sill"] == pytest.approx(1)
    assert run(history, "delete_placements", placements=[door.uid])["ok"]
    assert len(wall.ifc["openings"]) == 1 and len(wall.mesh.faces) == 10
    history.undo()
    assert len(wall.ifc["openings"]) == 2 and door in scene.groups


@pytest.mark.parametrize("occupied", [True, False])
def test_invalid_hosted_preview_stays_red_and_click_warns(occupied):
    from types import SimpleNamespace
    from built_core import ToolMachine, ViewportToolHost

    notices = []
    tool = SimpleNamespace()
    from tests.test_geometry_arrays import cube
    wall = cube()
    if occupied:
        wall.ifc["openings"] = [{"wall_rect": [.4, 0, .6, .4]}]
    view = SimpleNamespace(
        window=lambda: SimpleNamespace(notify=lambda *args: notices.append(args)),
        scene=SimpleNamespace(groups=[wall]),
        pick_face_placement=lambda *_: (wall.mesh.faces[2], wall),
        _pixel_to_ray=lambda *_: (V(.5 if occupied else .03, -1, .1), V(0,1,0)),
    )
    host = ViewportToolHost(view, None, tool)
    machine = ToolMachine("place_group")
    machine.set("load", ["family", [0,0,0], True, [[[-.1,0,0],[.1,0,.4]]]])
    machine.set("wall_family", "door")
    event = {"type": "hover", "px": (12,15)}
    machine.event(event, host)
    assert len(machine.preview_lines()) >= 4 and not notices
    assert tool.wireframe_color[0] > tool.wireframe_color[1] * 3
    assert not machine.event(dict(event, type="click"), host)["committed"]
    assert notices == [("warning", "Opening overlaps another opening or the wall edge.")]


def test_component_navigation_reuses_native_rows_until_document_edits(native_host):
    from tests.shell_isolation import attach_test_renderer
    from built_core import Matrix4
    from built_core.group import Group
    from built_core.mesh import Mesh

    attach_test_renderer(native_host)
    win, view = native_host.window, native_host.view
    scene = win.viewport.scene

    def add(name):
        mesh = Mesh()
        mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])
        group = Group(mesh, name=name)
        group.xform = Matrix4()
        scene.groups.append(group)
        scene.version += 1
        return group

    add("Alpha")
    add("Beta")
    L.open_library(win, "components")
    cs.before_frame(view, win)
    built = win._chrome_sheet.built
    rows = built._library_rows
    act(win, "focus", "cm:1")
    cs.before_frame(view, win)
    assert win._chrome_sheet.built is built
    assert L._state(view)["cursor"] == "cm:1"
    assert any(b.get("text") == "Beta" for b in built.wire()["detail"]["blocks"])
    cs.close(win)
    cs.before_frame(view, win)
    L.open_library(win, "components")
    cs.before_frame(view, win)
    assert win._chrome_sheet.built._library_rows is rows

    third = add("Gamma")
    cs.before_frame(view, win)
    added = win._chrome_sheet.built._library_rows
    assert added is not rows and L._state(view)["items"]["cm:2"]["name"] == "Gamma"
    scene.groups.remove(third)
    scene.version += 1
    cs.before_frame(view, win)
    assert win._chrome_sheet.built._library_rows is not added
    assert "cm:2" not in L._state(view)["items"]


def test_checked_definitions_select_replace_and_preserve_kept(win, monkeypatch):
    from built_core import Matrix4
    from built_core.group import Group
    from built_core.mesh import Mesh
    from built_core.kept_components import make_template

    scene = win.viewport.scene
    def component(name):
        mesh = Mesh()
        mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])
        group = Group(mesh, name=name)
        group.xform = Matrix4()
        group.ifc = {"class": "IfcBeam"}
        return group
    a, b, target = [component(name) for name in ("A", "B", "Target")]
    scene.groups.extend([a, b, target])
    kept = make_template(component("Retained"))
    scene.kept_components.append(kept)
    scene.version += 1
    L.open_library(win, "components")
    frame(win)
    act(win, "search", "class:IfcBeam used")
    entries = {t["label"]: t["id"] for t in tiles(frame(win))}
    assert set(entries) == {"A", "B", "Target"}
    for name in ("A", "B"):
        act(win, "bulk-check", entries[name])
        frame(win)
    act(win, "verb", "bulk-select")
    assert set(scene.selection) == {a, b}
    L.open_library(win, "components")
    frame(win)
    act(win, "search", "class:IfcBeam used")
    frame(win)
    for name in ("A", "B"):
        act(win, "bulk-check", entries[name])
        frame(win)
    act(win, "verb", "bulk-replace")
    frame(win)
    act(win, "focus", entries["Target"])
    frame(win)
    old_meshes = a.mesh, b.mesh
    revision = win.viewport.history.revision
    monkeypatch.setattr(win, "_ask_keep_orphans", lambda *_: False)
    act(win, "verb", "bulk-apply")
    assert a.mesh is target.mesh and b.mesh is target.mesh
    assert win.viewport.history.revision == revision + 1
    win.viewport.history.undo()
    assert (a.mesh, b.mesh) == old_meshes
    frame(win)
    act(win, "search", "unused")
    frame(win)
    act(win, "verb", "bulk-all")
    frame(win)
    act(win, "verb", "bulk-purge")
    review = detail(frame(win))
    assert any(["Retained", "Kept when unused", ""] in block.get("rows", []) for block in review)
    assert kept in scene.kept_components
    monkeypatch.setattr("built_render.chrome_dialogs.ask_later", lambda *args, **kw: args[4]("release"))
    revision = win.viewport.history.revision
    act(win, "verb", "bulk-release")
    assert kept not in scene.kept_components
    assert win.viewport.history.revision == revision + 1
    win.viewport.history.undo()
    assert kept in scene.kept_components


def test_collection_membership_and_unique_are_undoable(win, monkeypatch):
    from tests.test_replace_component import _component, _quad
    from built_core.group import copy_group

    scene = win.viewport.scene
    first = _component(_quad(), "Beam")
    second = copy_group(first)
    scene.groups.extend([first, second])
    scene.version += 1
    L.open_library(win, "components", search="source:model")
    row, = tiles(frame(win))
    act(win, "bulk-check", row["id"])
    frame(win)
    monkeypatch.setattr("built_render.chrome_forms.ask_text",
                        lambda *args, done, **kw: done("Structure"))
    act(win, "verb", "bulk-move")
    assert "Structure" in [side["label"] for side in frame(win)["side"]]
    win.viewport.history.undo()
    assert "Structure" not in [side["label"] for side in frame(win)["side"]]
    monkeypatch.setattr("built_render.chrome_forms.ask_choice",
                        lambda *args, done: done(next(label for label in args[3] if label.endswith(second.uid))))
    act(win, "verb", "unique-choose")
    assert first.mesh is not second.mesh
    win.viewport.history.undo()
    assert first.mesh is second.mesh


def test_large_component_index_filters_linked_files_without_rescanning(native_host, monkeypatch):
    from time import perf_counter
    from built_render import chrome_sheet
    from tests.shell_isolation import attach_test_renderer

    attach_test_renderer(native_host)
    win = native_host.window
    view = native_host.view
    provider = L.provider("components")
    rows = [{"key": f"fixture:{i}", "name": f"Component {i}",
             "source": "linked" if i % 3 == 0 else "model",
             "file": ("Stairs" if i % 2 == 0 else "Roof") if i % 3 == 0 else None,
             "classes": ["IfcBeam" if i % 2 == 0 else "IfcWall"],
             "uses": int(i % 5 != 0), "verbs": []} for i in range(3000)]
    calls = []
    monkeypatch.setattr(provider, "items", lambda ctx: calls.append(ctx) or rows)
    L.open_library(win, "components")
    chrome_sheet.before_frame(view, win)
    start = perf_counter()
    act(win, "search", "class:IfcBeam -source:linked used")
    chrome_sheet.before_frame(view, win)
    elapsed = (perf_counter() - start) * 1000
    sheet = win._chrome_sheet.built
    assert sheet.wire()["content"]["title"] == "800 definitions · 2200 excluded"
    assert {r["label"]: r["right"] for r in sheet.side if r.get("right")}["Stairs"] == "400"
    assert len(calls) == 1
    index = sheet._library_rows
    chrome_sheet.before_frame(view, win)
    assert win._chrome_sheet.built._library_rows is index
    print(f"3,000-definition host toggle: {elapsed:.2f} ms")
    import os
    if os.environ.get("BUILT_PERF_TESTS") == "1":
        assert elapsed < 30


def test_nested_linked_components_use_their_own_file_and_location(win, tmp_path):
    from built_core import links, Matrix4
    from built_core.group import Group
    from built_core.scene import Scene
    from formats import igz
    from tests.test_replace_component import _quad

    scene = win.viewport.scene
    for filename in ("Stairs", "Roof"):
        src = Scene()
        part = Group(_quad(), name=f"{filename} beam")
        part.xform = Matrix4()
        part.ifc = {"class": "IfcBeam"}
        src.groups.append(part)
        path = tmp_path / f"{filename}.built"
        igz.save_scene(src, path)
        scene.groups.append(links.make_link(scene, path, host_path=tmp_path / "host.built"))
    unavailable = str(tmp_path / "Unavailable.built")
    scene.link_errors = {unavailable: "Source file could not be loaded"}
    scene.version += 1
    L.open_library(win, "components", search="source:linked class:IfcBeam")
    sh = frame(win)
    assert {r["label"] for r in tiles(sh)} == {"Stairs beam", "Roof beam"}
    files = [r for r in sh["side"] if r.get("id", "").startswith("toggle:-file:")]
    assert {r["label"]: r["right"] for r in files} == {"Stairs.built": "1", "Roof.built": "1", "Unavailable.built": "?"}
    assert "Unavailable" in next(r["tip"] for r in files if r["right"] == "?")
    stairs = next(row for row in files if row["label"] == "Stairs.built")
    act(win, "side", stairs["id"])
    sh = frame(win)
    assert [r["label"] for r in tiles(sh)] == ["Roof beam"]
    assert any("Roof.built" in str(block.get("rows", [])) and "Unfiled" in str(block)
               for block in detail(sh))
    act(win, "search", "class:IfcBeam -source:linked")
    sh = frame(win)
    assert tiles(sh) == []
    roof = next(r for r in sh["side"] if r["label"] == "Roof.built")
    act(win, "side", roof["id"])
    assert [r["label"] for r in tiles(frame(win))] == ["Roof beam"]

def test_switches_only_request_the_active_provider(win, monkeypatch):
    calls = []
    for tab in ("materials", "components", "families"):
        provider = L.provider(tab)
        original = provider.items

        def items(ctx, original=original, tab=tab):
            calls.append(tab)
            return original(ctx)

        monkeypatch.setattr(provider, "items", items)

    L.open_library(win, "materials")
    frame(win)
    assert set(calls) == {"materials"}
    calls.clear()
    for tab in ("components", "families", "materials", "components", "families"):
        act(win, "tab", tab)
        frame(win)
    assert calls == ["components", "families"]


def test_open_and_edit_with_outstanding_usage_survey(win, monkeypatch):
    import threading
    import built_core

    real = built_core.library_usage
    allow = threading.Event()
    requested = []

    def held(scene, revision):
        requested.append(revision)
        assert allow.wait(5)
        return real(scene, revision)

    monkeypatch.setattr(built_core, "library_usage", held)
    scene = win.viewport.scene
    try:
        L.open_library(win, "materials")
        cs.before_frame(win.view, win)
        assert "Purge unused materials…" in str(win.view.renderer.sheet())
        scene.materials["Extra"] = Material("Extra")
        scene.version += 1
        cs.before_frame(win.view, win)
    finally:
        allow.set()
    assert "Purge unused materials (2)" in str(frame(win))
    assert requested[-1] == scene.content_version


def test_usage_index_distinguishes_front_and_back_paint(win):
    import built_core
    from built_core.group import Group

    scene = win.viewport.scene
    face = scene.mesh.faces[0]
    face.attrs["back"] = {"mat": "Concrete"}
    group = Group(name="Retained")
    group.material = {"mat": "Retained paint"}
    scene.materials["Retained paint"] = Material("Retained paint")
    scene.kept_components.append(group)
    scene.version += 1
    uses, _, _, _, unused = built_core.library_usage(scene, scene.content_version)
    assert uses == {"Beech": 1, "Concrete": 1, "Retained paint": 1}
    assert unused == len(built_core.purge_unused_materials(scene))


def test_image_stem_labels_preserve_material_identity_and_custom_names(win):
    scene = win.viewport.scene
    texture = {"path": "/textures/ae1ad9f06706c193-metal_dark.png"}
    for name in ("metal_dark", "architect_spec_A"):
        scene.materials[name] = Material(name, texture=texture)
    scene.version += 1
    L.open_library(win, "materials")
    labels = {item["id"]: item["label"] for item in tiles(frame(win))}
    assert labels["m:metal_dark"] == "Metal dark"
    assert labels["m:architect_spec_A"] == "architect_spec_A"
