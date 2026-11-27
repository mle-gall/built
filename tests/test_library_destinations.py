# SPDX-License-Identifier: GPL-3.0-or-later
"""Library destinations (docs/library.md): every saved kind goes to This
model, This project, My library or a shared library through one Save to
Library drawer; the browser shows where each item lives and copies, moves
and updates between destinations; Settings › Library names the folders; the
old saved folders migrate once. Everything stays inside the test profile:
My library's default folder is inside BUILT_SETTINGS_DIR."""
from __future__ import annotations

import json
import os
from pathlib import Path

import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from built_core import Vec3 as V
from built_core.materials import Material

from built_render import chrome_forms
from built_render import chrome_sheet as cs
from built_render import wgpu_library as L

KINDS = [k for k, _ in L.KINDS]
DATA = {
    "materials": {"name": "Oak", "color": [0.6, 0.4, 0.2]},
    "components": {"file": "component.built"},
    "profiles": {"format": "built.profile", "v": 1, "name": "Notch", "use": "profile", "sketch": {"curves": []}},
    "sections": {"format": "built.profile", "v": 1, "name": "Cutter", "use": "section", "sketch": {"curves": []}},
    "presets": {"name": "Rail", "primitive": "sweep", "values": {}, "record": {}, "locks": {}, "pack": "Mine"},
    "styles": {"name": "Clay", "face_mode": "shaded"},
    "scenes": {"camera": {}},
    "title_blocks": {"fields": []},
}


class Renderer:
    def __init__(self):
        self.calls = []

    def ui_set_large_sheet(self, text):
        cs.check_wire(text)
        self.calls.append(("sheet", json.loads(text) if text else None))

    def __getattr__(self, name):
        return lambda *a: self.calls.append((name, a))

    def sheet(self):
        return [c[1] for c in self.calls if c[0] == "sheet"][-1]


@pytest.fixture
def win(native_host, monkeypatch):
    w = native_host.window
    view = native_host.view
    monkeypatch.setattr(view, "renderer", Renderer())
    monkeypatch.setattr(view, "_ui_attached", True, raising=False)
    revealed = []
    monkeypatch.setattr("built_core.desktop_open.reveal", lambda path: revealed.append(path))
    w._revealed = revealed
    sc = w.viewport.scene
    sc.materials["Beech"] = Material("Beech", color=(0.75, 0.55, 0.35))
    f = sc.mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    f.attrs.update(sc.materials["Beech"].face_attrs())
    sc.version += 1
    return w


def frame(win):
    cs.before_frame(win.view, win)
    job = getattr(win.viewport.scene, "_library_usage_job", None)
    for _ in range(3):
        if not job:
            break
        job[1].result(timeout=5)
        cs.before_frame(win.view, win)
        if win.viewport.scene._library_usage_job is job:
            break
        job = win.viewport.scene._library_usage_job
    return win.view.renderer.sheet()


def act(win, verb, *args):
    cs.action(win, cs.SEP.join(("library", verb) + args))


def tiles(sh):
    return [i for b in sh["content"]["blocks"] if b["kind"] in ("rows", "tiles") for i in b["items"]]


def in_project(win, tmp_path, name="Site"):
    from built_core import project
    model = tmp_path / f"{name}-model.built"
    project.create(win.viewport.scene, tmp_path / f"{name}.builtproject", model)
    return tmp_path / f"{name}.builtproject"


def shared(tmp_path, name, read_only=False):
    folder = tmp_path / name.lower()
    folder.mkdir(exist_ok=True)
    L.set_shared(name, str(folder), read_only=read_only)
    return folder


def keys(kind, scene):
    return {(i["source"], i["name"]) for i in L.library_items(kind, scene)}


def test_every_kind_saves_to_every_writable_destination_and_reloads(win, tmp_path):
    in_project(win, tmp_path)
    team = shared(tmp_path, "Team")
    shared(tmp_path, "Vendor", read_only=True)
    scene = win.viewport.scene
    dests = {k: (label, root, writable) for k, label, root, writable, _ in L.destinations(scene)}
    assert list(dests) == ["model", "project", "mine", "shared:Team", "shared:Vendor"]
    assert dests["project"][1] == str(tmp_path / "Library")
    assert dests["mine"][1].startswith(os.environ["BUILT_SETTINGS_DIR"])      # never the real folder
    writable = [k for k, (_, _, w) in dests.items() if w]
    assert writable == ["model", "project", "mine", "shared:Team"]
    for kind in KINDS:
        for dest in writable:
            L.save_item(win, kind, dest, f"{kind} in {dest}", json.dumps(DATA[kind]))
        assert keys(kind, scene) == {(d, f"{kind} in {d}") for d in writable}
    # One file per item, by kind, with its id and revision.
    saved = json.loads(next((team / "styles").glob("*.json")).read_text())
    assert saved["format"] == "built.library" and saved["kind"] == "styles" and saved["rev"] == 1
    # Saving the same id again is its next revision.
    L.save_item(win, "styles", "shared:Team", "Clay 2", json.dumps(DATA["styles"]), id=saved["id"])
    again = json.loads((team / "styles" / f"{saved['id']}.json").read_text())
    assert (again["rev"], again["name"]) == (2, "Clay 2")


def test_read_only_shared_libraries_refuse_writes_clearly(win, tmp_path):
    vendor = shared(tmp_path, "Vendor", read_only=True)
    with pytest.raises(ValueError, match="Vendor is read-only"):
        L.save_item(win, "materials", "shared:Vendor", "Oak", json.dumps(DATA["materials"]))
    assert not any(vendor.iterdir())
    # Its items are listed, read-only: no move, edit or delete.
    (vendor / "materials").mkdir()
    entry = {"format": "built.library", "v": 1, "kind": "materials", "id": "vendoroak", "rev": 3, "name": "Vendor oak",
             "data": DATA["materials"]}
    (vendor / "materials" / "vendoroak.json").write_text(json.dumps(entry))
    row, = [i for i in L.library_items("materials", win.viewport.scene) if i["source"] == "shared:Vendor"]
    assert row["source_label"] == "Vendor"
    assert "copyto" in row["verbs"] and not {"moveto", "libedit", "libdelete"} & set(row["verbs"])
    # Not in a project: This project is greyed with why.
    project = [d for d in L.destinations(win.viewport.scene) if d[0] == "project"][0]
    assert project[3] is False and "not in a project" in project[4]


def test_model_items_survive_save_and_open(win, tmp_path):
    from built_core.scene import Scene
    from formats import igz
    scene = win.viewport.scene
    for kind in ("materials", "styles", "profiles"):
        L.save_item(win, kind, "model", f"{kind} kept", json.dumps(DATA[kind]), description="in the file", tags=["a", "b"])
    path = tmp_path / "kept.built"
    igz.save_scene(scene, path, backup=False)
    back = Scene()
    igz.load_into(back, path)
    for kind in ("materials", "styles", "profiles"):
        row, = L.library_items(kind, back)
        assert (row["source"], row["name"]) == ("model", f"{kind} kept")
        assert ["Tags", "a, b", ""] in row["defaults"] and ["Description", "in the file", ""] in row["defaults"]
    # A model item is one History step: Undo takes it out again.
    win.viewport.history.undo()
    assert [i["name"] for i in L.library_items("profiles", scene)] == []


def test_project_items_appear_in_another_model_of_the_project(win, tmp_path):
    from built_core import project
    from built_core.scene import Scene
    manifest = in_project(win, tmp_path)
    L.save_item(win, "presets", "project", "Shared rail", json.dumps(DATA["presets"]))
    other = Scene()                                                   # a second model of the project
    project.attach(other, manifest, tmp_path / "other.built")
    assert ("project", "Shared rail") in keys("presets", other)
    assert (tmp_path / "Library" / "presets").is_dir()


def test_settings_paths_change_and_persist(win, tmp_path):
    from built_core.settings import settings
    mine, default, rows = L.settings_state()
    assert mine == "" and default.endswith(str(Path("data") / "Built" / "Library"))
    elsewhere = tmp_path / "synced"
    L.set_mine(str(elsewhere))
    shared(tmp_path, "Team")
    shared(tmp_path, "Team", read_only=True)                       # changed, not added twice
    assert settings().value("library/root") == str(elsewhere)
    assert json.loads(settings().value("library/shared")) == [{"name": "Team", "path": str(tmp_path / "team"), "read_only": True}]
    assert L.settings_state()[0] == str(elsewhere)
    L.save_item(win, "materials", "mine", "Oak", json.dumps(DATA["materials"]))
    assert list((elsewhere / "materials").glob("*.json"))
    with pytest.raises(ValueError):
        L.set_mine("relative/path")
    with pytest.raises(ValueError):
        L.set_shared("My library", str(tmp_path))
    assert L.remove_shared("Team") and L.settings_state()[2] == []
    L.set_mine("")
    assert L.settings_state()[0] == ""


def test_migration_copies_old_saved_items_once_and_keeps_them(tmp_path, monkeypatch):
    from built_core.settings import settings
    from built_render.wgpu_library import ProfilesProvider
    monkeypatch.setenv("BUILT_SETTINGS_DIR", str(tmp_path / "profile"))
    old = Path(ProfilesProvider.saved_directory())
    old.mkdir(parents=True)
    (old / "Notch.builtprofile").write_text(json.dumps(DATA["profiles"]))
    (old / "Cutter.builtprofile").write_text(json.dumps(DATA["sections"]))
    presets = old.with_name("presets")
    presets.mkdir()
    (presets / "rail.builtpreset").write_text(json.dumps(dict(DATA["presets"], uid="00000001000000aa", rev=2)))
    st = settings()
    st.setValue("library/mine", json.dumps([{"type": "materials", "name": "Oak", "color": [1, 0, 0], "category": "Wood"}]))
    st.setValue("styles/user", json.dumps([{"name": "Clay", "face_mode": "shaded"}]))
    assert L.migrate() == 4
    root = Path(L.settings_state()[1])
    assert {p.parent.name for p in root.glob("*/*.json")} == {"profiles", "sections", "presets", "materials", "styles"}
    assert (old / "Notch.builtprofile").exists() and (presets / "rail.builtpreset").exists()
    assert L.migrate() is None                                        # once
    assert ("mine", "Oak") in keys("materials", None)
    preset, = L.library_items("presets", None)
    assert (preset["lib"]["id"], preset["lib"]["rev"]) == ("00000001000000aa", 2)
    # The old rows are not listed twice beside their copies.
    from types import SimpleNamespace
    names = [i["name"] for i in ProfilesProvider().items(SimpleNamespace(scene=None))]
    assert names.count("Notch") == 1


def test_save_drawer_lists_destinations_and_remembers_the_last_one(win, tmp_path, monkeypatch):
    shown = []
    monkeypatch.setattr(chrome_forms, "show", lambda parent, form: shown.append(form))
    shared(tmp_path, "Vendor", read_only=True)
    L.save_to_library(win, "styles", "Clay", json.dumps(DATA["styles"]))
    form, = shown
    assert form.placement == "drawer"
    dest = form.field("destination")
    labels = [o[1] for o in dest.options]
    # Writable first (This model, My library), then the greyed ones with why.
    assert labels[:2] == ["This model", "My library"] and dest.choice_limit == 2
    assert any(lbl.startswith("This project — ") for lbl in labels[2:])
    assert any(lbl.startswith("Vendor — ") for lbl in labels[2:])
    assert form.value("destination") == "mine"                      # first save: My library
    form.on_done("ok", dict(form.values(), name="Clay", description="Warm", tags="soft, soft, matte", destination="model"))
    row, = L.library_items("styles", win.viewport.scene)
    assert row["source"] == "model" and ["Tags", "soft, matte", ""] in row["defaults"]
    L.save_to_library(win, "styles", "Clay", json.dumps(DATA["styles"]))
    assert shown[-1].value("destination") == "model"                # remembered per kind
    L.save_to_library(win, "materials", "Oak", json.dumps(DATA["materials"]))
    assert shown[-1].value("destination") == "mine"


def test_material_saves_with_its_image_and_paints_from_my_library(win, tmp_path, monkeypatch):
    import zlib
    import struct
    shown = []
    monkeypatch.setattr(chrome_forms, "show", lambda parent, form: shown.append(form))
    image = tmp_path / "oak.png"
    raw = b"".join(b"\0" + bytes((120, 80, 40)) * 4 for _ in range(4))
    chunk = lambda tag, data: struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data))
    image.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 4, 4, 8, 2, 0, 0, 0))
                      + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
    sc = win.viewport.scene
    sc.materials["Oak"] = Material("Oak", color=(0.6, 0.4, 0.2), texture={"path": str(image)})
    sc.version += 1
    L.open_library(win, "materials")
    frame(win)
    act(win, "menu", "m:Oak", "keep")
    form = shown[-1]
    assert "model" not in [o[0] for o in form.field("destination").options]   # it is in the model already
    form.on_done("ok", dict(form.values(), destination="mine"))
    row, = [i for i in L.library_items("materials", sc) if i["source"] == "mine"]
    copied = Path(row["recipe"]["texture"]["path"])
    assert copied.is_file() and copied.parent.name.endswith(".files") and copied != image
    assert row["base"] == "image"
    picked = []
    monkeypatch.setattr(L, "_pick_paint", lambda w, **kw: picked.append(kw["recipe"]))
    sh = frame(win)
    act(win, "side", "s:mine")
    sh = frame(win)
    tile, = [t for t in tiles(sh) if t["id"] == row["key"]]
    assert tile["place"] == "My library"
    act(win, "open", row["key"])
    assert picked[0]["texture"]["path"] == str(copied)


def test_copy_move_update_reveal_and_delete_between_destinations(win, tmp_path, monkeypatch):
    team = shared(tmp_path, "Team")
    asked = []
    monkeypatch.setattr(chrome_forms, "ask_choice",
                        lambda parent, title, label, items, current=0, done=None, **kw: (asked.append((title, list(items))), done(pick[0])))
    monkeypatch.setattr("built_render.chrome_dialogs.ask_later",
                        lambda parent, title, body, buttons, done, **kw: done("yes"))
    sc = win.viewport.scene
    ident = L.save_item(win, "styles", "mine", "Clay", json.dumps(DATA["styles"]))
    L.open_library(win, "styles")
    frame(win)
    key = f"saved:styles:{ident}:mine"
    pick = ["Team"]
    act(win, "menu", key, "copyto")
    assert asked[-1] == ("Copy to", ["This model", "Team"])
    assert keys("styles", sc) == {("mine", "Clay"), ("shared:Team", "Clay")}
    # A newer revision in My library is offered as an update in Team.
    L.save_item(win, "styles", "mine", "Clay", json.dumps(dict(DATA["styles"], face_mode="xray")), id=ident)
    team_row, = [i for i in L.library_items("styles", sc) if i["source"] == "shared:Team"]
    assert "update" in team_row["verbs"] and team_row["lib"]["update_from"] == "mine"
    frame(win)
    act(win, "menu", team_row["key"], "update")
    saved = json.loads((team / "styles" / f"{ident}.json").read_text())
    assert saved["rev"] == 2 and saved["data"]["face_mode"] == "xray"
    # Move into the model takes it out of Team.
    pick = ["This model"]
    frame(win)
    act(win, "menu", team_row["key"], "moveto")
    assert keys("styles", sc) == {("mine", "Clay"), ("model", "Clay")}
    assert not (team / "styles" / f"{ident}.json").exists()
    act(win, "menu", key, "reveal")
    assert win._revealed[-1].endswith(f"{ident}.json")
    frame(win)
    act(win, "menu", key, "libdelete")
    assert keys("styles", sc) == {("model", "Clay")}


def test_styles_tab_and_display_panel_use_saved_styles(win, monkeypatch):
    L.save_item(win, "styles", "mine", "Clay", json.dumps(DATA["styles"]))
    L.open_library(win, "styles")
    frame(win)
    act(win, "side", "s:bundled")
    names = {t["label"]: t.get("place", "") for t in tiles(frame(win))}
    assert names["Default"] == "Built"
    act(win, "side", "s:mine")
    sh = frame(win)
    assert [(t["label"], t["place"]) for t in tiles(sh)] == [("Clay", "My library")]
    used = []
    monkeypatch.setattr(win, "set_active_style", lambda style: used.append(style.name))
    act(win, "open", tiles(sh)[0]["id"])
    assert used == ["Clay"]


def test_saved_sections_stay_out_of_the_sweep_pickers(win):
    from types import SimpleNamespace
    from built_render.wgpu_library import ProfilesProvider
    L.save_item(win, "sections", "mine", "Cutter", json.dumps(DATA["sections"]))
    L.save_item(win, "profiles", "mine", "Notch", json.dumps(DATA["profiles"]))
    picker = [i["name"] for i in ProfilesProvider().items(SimpleNamespace(scene=win.viewport.scene))]
    assert "Notch" in picker and "Cutter" not in picker
    L.open_library(win, "profiles")
    frame(win)
    act(win, "side", "s:mine")
    labels = {t["label"] for t in tiles(frame(win))}
    assert {"Notch", "Cutter"} <= labels


def test_settings_library_pane_sets_folders_and_shared_libraries(win, tmp_path, monkeypatch):
    shown = []
    monkeypatch.setattr(chrome_forms, "show", lambda parent, form: shown.append(form))

    def settings(verb, *args):
        cs.action(win, cs.SEP.join(("settings", verb) + args))
        cs.before_frame(win.view, win)
        return win.view.renderer.sheet()

    win.open_preferences()
    cs.before_frame(win.view, win)
    sheet = settings("side", "library")
    blocks = sheet["content"]["blocks"]
    assert [b["text"] for b in blocks if b["kind"] == "caption"] == ["My library", "Shared libraries"]
    default = L.settings_state()[1]
    assert any(row[1] == default for b in blocks if b["kind"] == "kv" for row in b["rows"])
    team = tmp_path / "team"
    team.mkdir()
    settings("verb", "lib_add")
    form = shown[-1]
    assert form.placement == "drawer" and [f.id for f in form.fields] == ["name", "folder", "read_only"]
    form.on_done("ok", dict(name="Team", folder=str(team), read_only=True))
    sheet = settings("side", "library")
    rows = [row for b in sheet["content"]["blocks"] if b["kind"] == "kv" for row in b["rows"]]
    assert ["Team", f"{team} · Read-only"] in [list(r[:2]) for r in rows]
    assert L.settings_state()[2] == [("Team", str(team), True)]
    settings("verb", "lib_edit:0")
    shown[-1].on_done("ok", dict(name="Team drive", folder=str(team), read_only=False))
    assert L.settings_state()[2] == [("Team drive", str(team), False)]
    synced = tmp_path / "synced"
    settings("verb", "lib_root")
    shown[-1].on_done("ok", dict(folder=str(synced)))
    assert L.settings_state()[0] == str(synced)
    settings("verb", "lib_root_reveal")
    assert win._revealed[-1] == str(synced)
    settings("verb", "lib_root_default")
    settings("verb", "lib_remove:0")
    assert L.settings_state()[0] == "" and L.settings_state()[2] == []
