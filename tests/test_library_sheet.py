# SPDX-License-Identifier: GPL-3.0-or-later
"""The Library on the large sheet (views/wgpu_library.py, option B with C's
palette): types as tabs, scopes and categories on the side with the linked
files' materials read-only, 96-pt tiles, the details pane and its editor;
the Paint strip's fast path and Browse all…; the dock's inspector editing
the material Paint holds. Offscreen: a stand-in renderer records the
sheet's JSON."""
from __future__ import annotations

import json

import pytest
from PySide6.QtCore import QSettings
from ingetrazo_core import Vec3 as V
from PySide6.QtWidgets import QApplication

from core.materials import Material
from views import chrome_sheet as cs
from views import wgpu_library as L

_app = QApplication.instance() or QApplication([])


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


class View:
    def __init__(self, win):
        self.renderer = Renderer()
        self._ui_attached = True
        self._win = win

    def window(self):
        return self._win

    def update(self):
        pass

    def setFocus(self, *_):
        pass


@pytest.fixture(autouse=True)
def _throwaway_settings(tmp_path, monkeypatch):
    QSettings.setDefaultFormat(QSettings.IniFormat)
    for scope in (QSettings.UserScope, QSettings.SystemScope):
        QSettings.setPath(QSettings.IniFormat, scope, str(tmp_path))
    QSettings().clear()
    monkeypatch.setenv("INGETRAZO_STORE", "0")


@pytest.fixture
def win(monkeypatch):
    from views.main_window import MainWindow
    w = MainWindow(defer_extensions=True)
    view = View(w)
    w._doc.stand_in = view
    monkeypatch.setattr(cs, "available", lambda _w: True)
    monkeypatch.setattr(cs, "_view", lambda _w: view)
    monkeypatch.setattr("views.wgpu_ui._window", lambda v: w)
    sc = w.viewport.scene
    sc.materials["Beech"] = Material("Beech", color=(0.75, 0.55, 0.35))
    sc.materials["Concrete"] = Material("Concrete", color=(0.6, 0.6, 0.6))
    f = sc.mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    f.attrs.update(sc.materials["Beech"].face_attrs())
    sc.version += 1
    w._view_ = view
    yield w
    for d in w.documents():
        d.saved_content = d.viewport.scene.content_version
    w._saved_version = w.viewport.scene.version
    w.close()


def frame(win):
    cs.before_frame(win._view_, win)
    return win._view_.renderer.sheet()


def act(win, verb, *args):
    cs.action(win, cs.SEP.join(("library", verb) + args))


def tiles(sh):
    return [i for b in sh["content"]["blocks"] if b["kind"] == "tiles" for i in b["items"]]


def detail(sh):
    return sh["detail"]["blocks"]


def test_browse_all_opens_the_sheet_on_the_models_materials(win):
    assert L.open_library(win, "materials")
    sh = frame(win)
    assert sh["id"] == "library"
    assert [(t["id"], t["on"], t["enabled"]) for t in sh["tabs"]] == [
        ("materials", True, True), ("components", False, True), ("profiles", False, False)]
    assert sh["search"]["placeholder"] == "Search all materials"
    side = [(s["id"], s["label"]) for s in sh["side"]]
    assert side[:4] == [("s:model", "In this model"), ("s:recent", "Recently used"), ("s:mine", "My library"),
                        ("s:bundled", "Bundled")]
    assert any(i.startswith("c:bundled:") for i, _ in side)          # its categories
    assert sh["content"]["title"] == "In this model"
    t = tiles(sh)
    assert [x["label"] for x in t] == ["Beech", "Concrete"]
    assert t[0]["sub"] == "colour · used by 1" and t[0]["hex"]
    assert next(b for b in sh["content"]["blocks"] if b["kind"] == "tiles")["size"] == 96
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
    act(win, "headmenu", "sort", "facet:image")
    assert tiles(frame(win)) == []                                    # no image bases in this model
    act(win, "headmenu", "sort", "facet:all")
    act(win, "headmenu", "sort", "used")
    assert L._state(win._view_)["sort"] == "used"
    assert frame(win)["menus"][0]["label"] == "Sort: Most used"


def test_the_editor_applies_edits_and_sliders_when_they_settle(win, monkeypatch):
    L.open_library(win)
    sh = frame(win)
    form = next(b for b in detail(sh) if b["kind"] == "fields")
    ids = [f[0] for f in form["fields"]]
    assert ids[:3] == ["mat.base", "mat.color", "mat.pbr"] and "mat.opacity" in ids and "mat.name" in ids
    sc = win.viewport.scene
    act(win, "v", "mat.color", "#336699")                             # the picker's Apply
    assert L._hex(sc.materials["Beech"].color) == "#336699"
    act(win, "v", "mat.pbr", "1")                                     # PBR on: its sliders come
    sh = frame(win)
    ids = [f[0] for f in next(b for b in detail(sh) if b["kind"] == "fields")["fields"]]
    assert "mat.roughness" in ids and "mat.map.roughness_map" in ids
    assert sc.materials["Beech"].pbr == {"metallic": 0.0, "roughness": 0.5}
    prov = L._sheet_provider(win._view_)
    for v in ("40", "55", "62"):                                      # a drag: one step when it settles
        act(win, "v", "mat.roughness", v)
    sh = frame(win)
    slider = next(f for b in detail(sh) if b["kind"] == "fields" for f in b["fields"] if f[0] == "mat.roughness")
    assert slider[4] == 62.0                                          # shows what is dragged meanwhile
    assert sc.materials["Beech"].pbr["roughness"] == 0.5
    prov._timer.timeout.emit()
    assert sc.materials["Beech"].pbr["roughness"] == pytest.approx(0.62)
    picked = []
    monkeypatch.setattr(L, "_pick_image", lambda w, title: (picked.append(title), None)[1])
    act(win, "v", "mat.base", "1")                                    # Image: asks for a picture
    assert picked == ["Base image"]
    act(win, "v", "mat.pbr", "0")
    assert sc.materials["Beech"].pbr is None


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
    assert L._state(win._view_)["recent"][0] == key
    # Recently used lists it, and the strip offers it next to the model's.
    L.open_library(win)
    frame(win)
    act(win, "side", "s:recent")
    assert [x["id"] for x in tiles(frame(win))] == [key]


def test_delete_asks_first_over_the_sheet(win, monkeypatch):
    asked = []
    monkeypatch.setattr("views.chrome_dialogs.ask_later",
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
    from core import links
    from core.materials import Material as M
    from formats import igz
    files = cf.build(tmp_path / "comp", big=12)
    # Paint the source's parts, then link it into the open document.
    from core.scene import Scene
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
    act(win, "side", "s:bundled")
    sh = frame(win)
    caps = [b["text"] for b in sh["content"]["blocks"] if b["kind"] == "caption"]
    assert "People (2D)" in caps and "Starters (3D)" in caps
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
    view = win._view_
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
    view = win._view_
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
