# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise the Start provider and its actions with an offscreen chrome renderer."""
from __future__ import annotations

import json
from types import SimpleNamespace

from tests.shell_isolation import native_host, restore_shell_patches  # noqa: F401
import pytest
import numpy as np
from built_core import Input as Qt
from built_render import KeyEvent, SurveyRaster, blank_picture
from tests.raster_samples import pixels, solid_raster

from built_core.settings import settings
from built_core import recent
from built_core.scene import Scene
from formats import igz
from built_render import chrome_sheet as cs
from built_render import wgpu_start as ws


class FakeRenderer:
    def __init__(self):
        self.calls = []

    def ui_set_large_sheet(self, text):
        cs.check_wire(text)       # what the chrome reads (ui/large_sheet.rs)
        self.calls.append(("sheet", json.loads(text) if text else None))

    def ui_set_icons(self, icons):
        self.calls.append(("icons", [i[0] for i in icons]))

    def ui_claims_key(self, name, mods=0):
        return True

    def sheet(self):
        return [c[1] for c in self.calls if c[0] == "sheet"][-1]


class FakeView:
    def __init__(self, win):
        self.renderer = FakeRenderer()
        self._ui_attached = True
        self._win = win
        self.updates = 0
        self.focused = 0

    def window(self):
        return self._win

    def update(self):
        self.updates += 1

    def setFocus(self, *_):
        self.focused += 1


@pytest.fixture
def win(monkeypatch, native_host):
    st = settings()
    for k in ("home/recent_files", "home/recent_projects", "home/last_file", "home/meta", "recent_files"):
        st.remove(k)
    ws.forget_cache()
    w = native_host.window
    view = FakeView(w)
    monkeypatch.setattr(ws, "_view", lambda _w: view)
    monkeypatch.setattr(cs, "_view", lambda _w: view)
    monkeypatch.setattr("views.wgpu_ui._window", lambda v: w)
    w._fake_view = view
    yield w
    p = w.__dict__.get("_start_page")
    if p is not None and p.dog is not None:
        p.dog.stop()


def thumb_png(w=40, h=24):
    return gradient(w, h).png()


def doc(tmp_path, name, picture=True):
    scene = Scene()
    if picture:
        scene._home_thumbnail_png = thumb_png()
    path = tmp_path / name
    igz.save_scene(scene, path)
    return path


def frame(win):
    """One frame: the page, then the sheet it is on. Returns the renderer."""
    view = win._fake_view
    ws.before_frame(view, win)
    cs.before_frame(view, win)
    return view.renderer


def wait_loader(win):
    p = ws.page(win)
    if p.loader is not None:
        p.loader.join(5)
    p._poll()


def sheet(win):
    return frame(win).sheet()


def section(sh, key):
    """The rows of one list section (Continue, Pinned, Recent…), [] if none."""
    return next((b["items"] for b in sh["content"]["blocks"] if b["kind"] == "rows" and b.get("id") == key), [])


def detail(sh):
    return (sh.get("detail") or {}).get("blocks") or []


def act(win, verb, *args):
    cs.action(win, cs.SEP.join(("start", verb) + args))


# ---- Shown or not ------------------------------------------------------------
def test_offscreen_opens_start_sheet(win):
    ws.show(win)
    assert frame(win).sheet()["id"] == "start"


def test_the_window_opens_on_it_only_without_a_file(monkeypatch):
    assert ws.wants_at_start(False)
    assert not ws.wants_at_start(True)
    monkeypatch.setenv("BUILT_START", "0")
    assert not ws.wants_at_start(False)


def test_show_home_puts_the_page_on_the_sheet(win, monkeypatch, tmp_path):
    a, b = doc(tmp_path, "alpha.built"), doc(tmp_path, "beta.built", picture=False)
    recent.remember(a)
    recent.remember(b)
    recent.note(a, {"project": "Ride", "links": 2, "imported": "skp", "unfiled": 5,
                    "problems": 13, "workspace": "Model"})
    ws.show(win)
    assert ws.shown(win) and cs.current(win) is ws.page(win).sheet
    assert getattr(win, "_home", None) is None          # no Qt dialog
    sh = sheet(win)
    assert sh["id"] == "start" and sh["closable"] is False   # blank untitled behind: no return
    assert [v["id"] for v in sh["verbs"]] == ["openfile"]
    assert sh["menus"] == []
    assert [t["label"] for t in sh["tabs"]] == ["Models", "Projects"]
    assert sh["search"]["placeholder"]
    wait_loader(win)
    sh = sheet(win)
    beta, alpha = section(sh, "recent")
    assert beta["label"] == "beta.built"
    assert alpha["label"] == "alpha.built" and alpha["badge"] == "13"
    assert alpha["sub"].startswith("Ride")
    assert alpha["icon"].startswith("thumb:")           # its picture, uploaded once
    assert "icon" not in beta
    assert any(c[0] == "icons" and alpha["icon"] in c[1] for c in frame(win).calls)
    tpl = section(sh, "templates")
    assert [t["id"] for t in tpl] == ["tpl:blank", "tpl:survey", "tpl:steel", "tpl:arch_mm"]
    assert sh["footer"]["hint"] == ""
    assert [l["id"] for l in sh["footer"]["links"]] == ["libraries", "prefs", "extensions", "help"]
    # The first row is focused; its preview is on the right.
    assert sh["focus"] == beta["id"]


def test_the_preview_shows_saved_metadata_without_opening(win, monkeypatch, tmp_path):
    a = doc(tmp_path, "job.built")
    recent.remember(a)
    recent.note(a, {"project": "Park", "problems": 13, "workspace": "Model › North", "unfiled": 50})
    slot = tmp_path / "slot.built"
    slot.write_bytes(a.read_bytes())
    import os
    os.utime(slot, (a.stat().st_mtime + 60, a.stat().st_mtime + 60))
    monkeypatch.setattr("built_core.autosave.pending", lambda p, *k, **kw: slot)
    ws.show(win)
    wait_loader(win)
    sh = sheet(win)
    blocks = detail(sh)
    pic = next(b for b in blocks if b["kind"] == "picture")
    assert pic["icon"].startswith("thumb:")
    # The row and the preview share one picture, sent once: two loads of a
    # key between frames crashed egui-wgpu (a texture freed mid-upload).
    sent = [k for c in frame(win).calls if c[0] == "icons" for k in c[1]]
    assert sent.count(pic["icon"]) == 1
    kv = dict((r[0], r[1]) for b in blocks if b["kind"] == "kv" for r in b["rows"])
    assert kv["Units"] == "m"
    assert kv["File size"] and kv["Last saved"]
    assert win._doc.path is None
    assert any(b["kind"] == "text" and "Auto-save is newer" in b["text"] for b in blocks)
    assert sh["footer"]["action"]["label"] == "Open model"


def test_the_preview_lists_links_and_their_state(win, monkeypatch, tmp_path):
    """The synthetic composite: Host links A (which links C and D) and B.
    Before opening, the preview says which links are found, changed on
    disk or missing."""
    from tests import composite_fixture as cf
    files = cf.build(tmp_path / "comp", big=40)
    host = files["host"]
    import os
    t = host.stat().st_mtime
    os.utime(files["b"], (t + 120, t + 120))            # B changed since Host was saved
    files["d"].unlink()                                 # D is gone
    recent.remember(host)
    ws.show(win)
    wait_loader(win)
    sh = sheet(win)
    blocks = detail(sh)
    cap = next(b for b in blocks if b["kind"] == "caption" and b["text"] == "Linked files")
    assert cap["right"] == "4 · 2 to look at"
    rows = next(b for b in blocks if b["kind"] == "rows")["items"]
    got = [(r["label"], r.get("depth", 0), r["dot"], r.get("dot_text", "")) for r in rows]
    assert got == [("A.igz", 0, "ok", "Found"), ("C.igz", 1, "ok", "Found"), ("D.igz", 1, "error", "Missing"),
                   ("B.igz", 0, "warn", "Changed on disk")]
    assert all(r.get("fixed") for r in rows)            # read, not focused


def test_first_frame_never_waits_for_pictures(win, monkeypatch, tmp_path):
    a = doc(tmp_path, "alpha.built")
    recent.remember(a)
    gate = __import__("threading").Event()
    real = ws._load_one
    monkeypatch.setattr(ws, "_load_one", lambda p: (gate.wait(5), real(p))[1])
    ws.show(win)
    cont = section(sheet(win), "recent")                  # drawn before the worker is done
    assert "icon" not in cont[0]
    gate.set()
    wait_loader(win)
    assert section(sheet(win), "recent")[0]["icon"].startswith("thumb:")


def test_typing_filters_the_list(win, monkeypatch, tmp_path):
    for n in ("ride.built", "site.built", "kiosk.built"):
        recent.remember(doc(tmp_path, n))
    ws.show(win)
    wait_loader(win)
    sheet(win)
    act(win, "search", "kio")
    sh = sheet(win)
    names = [r["label"] for k in ("cont", "recent") for r in section(sh, k)]
    assert names == ["kiosk.built"]
    assert section(sh, "templates") == []
    act(win, "search", "zzz")
    assert "Nothing matches" in sheet(win)["content"]["empty"]


# ---- Keys --------------------------------------------------------------------
def key_event(key, text="", mods=Qt.NoModifier):
    return KeyEvent(True, key, mods, text)


def test_keys_go_to_the_sheet_but_command_keys_stay_the_windows(win, monkeypatch):
    view = win._fake_view
    assert cs.claims_key(view, key_event(Qt.Key_Escape)) is None    # not shown
    ws.show(win)
    assert cs.claims_key(view, key_event(Qt.Key_Escape))
    assert cs.claims_key(view, key_event(Qt.Key_A, "a"))
    assert cs.claims_key(view, key_event(Qt.Key_Down))
    assert cs.claims_key(view, key_event(Qt.Key_F, "f", Qt.ControlModifier))   # ⌘F: the search
    # ⌘O, ⌘N, ⌘K: the window's.
    assert cs.claims_key(view, key_event(Qt.Key_O, "o", Qt.ControlModifier)) is False
    assert cs.claims_key(view, key_event(Qt.Key_K, "k", Qt.ControlModifier)) is False


def test_escape_returns_only_to_an_open_document(win, monkeypatch, tmp_path):
    ws.show(win)
    assert sheet(win)["closable"] is False      # only the blank untitled behind: Esc stays
    ws.action(win, "close")
    assert ws.shown(win)
    a = doc(tmp_path, "open.built")
    assert win.open_path(a)                 # (opening dismisses it)
    frame(win)
    assert not ws.shown(win)
    ws.show(win)
    sh = sheet(win)
    assert sh["closable"] is True
    assert section(sh, "recent")[0]["label"] == a.name
    act(win, "close")                       # Esc on the sheet
    assert not ws.shown(win) and not cs.shown(win)
    assert frame(win).calls[-1] == ("sheet", None)


# ---- Actions -----------------------------------------------------------------
def test_enter_on_a_row_opens_it_through_the_window(win, monkeypatch, tmp_path):
    a, b = doc(tmp_path, "tile.built"), doc(tmp_path, "other.built")
    recent.remember(a)
    recent.remember(b)
    opened = []
    real = win.open_path
    monkeypatch.setattr(win, "open_path", lambda p: (opened.append(p), real(p))[1])
    ws.show(win)
    sheet(win)
    act(win, "open", "file:" + str(a.resolve()))
    assert opened == [a.resolve()]
    assert win._doc.path.resolve() == a.resolve()
    frame(win)
    assert not ws.shown(win)
    assert settings().value("home/last_file") == str(a.resolve())


def test_a_moved_file_is_located(win, monkeypatch, tmp_path):
    from built_render.shell_files import ShellFileDialog
    moved = doc(tmp_path, "moved.built")
    old = tmp_path / "gone.built"
    recent.remember(old)
    monkeypatch.setattr(ShellFileDialog, "getOpenFileName", lambda *a: (str(moved), ""))
    ws.show(win)
    sheet(win)
    act(win, "menu", "file:" + str(old.resolve()), "locate")
    assert win._doc.path.resolve() == moved.resolve()
    assert str(old.resolve()) not in [r["path"] for r in recent.entries()]


def test_pin_toggles_and_pinned_come_first(win, monkeypatch, tmp_path):
    a, b, c = doc(tmp_path, "a.built"), doc(tmp_path, "b.built"), doc(tmp_path, "c.built")
    for p in (a, b, c):
        recent.remember(p)
    ws.show(win)
    sheet(win)
    act(win, "menu", "file:" + str(a.resolve()), "pin")
    rows = recent.entries()
    assert rows[0] == {"path": str(a.resolve()), "pinned": True}
    pinned = section(sheet(win), "pinned")
    assert [r["id"] for r in pinned] == ["file:" + str(a.resolve())]
    assert ("pin", "Unpin") in [tuple(m) for m in pinned[0]["menu"]]
    act(win, "menu", "file:" + str(a.resolve()), "pin")
    assert section(sheet(win), "pinned") == []
    # Pinning is not opening: Continue stays on the last document opened.
    assert settings().value("home/last_file") == str(c.resolve())


def test_new_model_and_templates(win, monkeypatch):
    monkeypatch.setattr("built_render.units_question.offer_at_start", lambda w: pytest.fail("unexpected units modal"))
    ws.show(win)
    sh = sheet(win)
    assert sh["focus"] == "tpl:blank" and sh["footer"]["action"]["label"] == "New model"
    act(win, "v", "units", "1")
    act(win, "verb", "open")
    assert win.viewport.scene.units["length"] == "cm"
    assert not ws.shown(win)
    ws.show(win)
    sheet(win)
    act(win, "open", "tpl:steel")
    assert not ws.shown(win)
    scene = win.viewport.scene
    from core import bim_registry
    reg = bim_registry.get(scene)
    assert reg["structure"]["nodes"]["steel.beams"]["recipe"]["class"] == "IfcBeam"
    assert reg["structure"]["active"] == "steel.beams"
    assert scene.units["length"] == "mm"
    assert win._doc_is_blank()              # a template is where it starts, not an edit
    # A template row: its preview and Enter.
    ws.show(win)
    sheet(win)
    act(win, "focus", "tpl:survey")
    blocks = detail(sheet(win))
    assert next(b for b in blocks if b["kind"] == "heading")["text"] == "As-built survey"
    act(win, "open", "tpl:arch_mm")
    assert bim_registry.get(win.viewport.scene)["structure"]["active"] == "building.walls"


@pytest.mark.parametrize("tid", [t[0] for t in ws.TEMPLATES if t[3]])
def test_every_template_structure_is_valid(tid):
    from core import bim_registry
    t = next(t for t in ws.TEMPLATES if t[0] == tid)
    scene = Scene()
    assert ws.apply_structure(scene, t[3], t[4])
    assert not bim_registry.validate(bim_registry.get(scene))


def test_a_document_coming_in_closes_the_page(win, monkeypatch, tmp_path):
    ws.show(win)
    frame(win)
    a = doc(tmp_path, "dropped.built")
    assert win._host.open_path(a)  # An OS open reaches the host directly.
    frame(win)
    assert not ws.shown(win)


def test_settings_from_the_footer_goes_over_it_and_the_page_comes_back(win, monkeypatch):
    ws.show(win)
    sheet(win)
    other = type("Other", (cs.Provider,), {"id": "other", "build": lambda self, ctx: cs.Sheet("other", "Other")})()
    assert cs.open(win, other)
    assert sheet(win)["id"] == "other"
    act_other = cs.SEP.join(("other", "close"))
    cs.action(win, act_other)
    frame(win)                                          # the sheet goes…
    assert sheet(win)["id"] == "start"                  # …and the page is back under it


def test_missing_frame_closes_start_with_a_message(win, monkeypatch):
    monkeypatch.setattr(ws, "_view", lambda _w: None)
    ws.show(win)
    ws.page(win)._watchdog()
    assert not ws.shown(win)
    assert "could not draw" in win.statusBar().currentMessage()


def test_note_remembers_what_the_window_knows(win, tmp_path):
    a = doc(tmp_path, "noted.built")
    assert win.open_path(a)
    m = recent.meta(a)
    assert m["workspace"] == "Model" and m["links"] == 0
    raw = json.loads(settings().value("home/meta"))
    assert str(a.resolve()) in raw


# ---- Pictures: a black one is no picture ---------------------------------------
def gradient(w=280, h=175):
    rgba = np.empty((h, w, 4), np.uint8)
    rgba[:] = (200, 214, 230, 255)                # sky
    rgba[h // 2:] = (120, 128, 110, 255)          # ground
    rgba[h // 4:3 * h // 4, w // 3:2 * w // 3] = (70, 70, 80, 255)
    return SurveyRaster(w, h, rgba.tobytes())


def test_blank_pictures_are_told_apart():
    black = solid_raster(280, 175, (0, 0, 0))
    grey = solid_raster(280, 175, (128, 128, 128))
    for image in (black, grey):
        assert blank_picture(image.width, image.height, image.rgba())
    speck = pixels(black).copy()
    speck[86:89, 138:142] = 255
    assert blank_picture(black.width, black.height, speck.tobytes())
    assert blank_picture(0, 0, b"")
    image = gradient()
    assert not blank_picture(image.width, image.height, image.rgba())


def test_a_black_thumbnail_shows_the_placeholder(win, monkeypatch, tmp_path):
    black = solid_raster(280, 175, (0, 0, 0))
    dark = tmp_path / "dark.built"
    scene = Scene()
    scene._home_thumbnail_png = black.png()
    igz.save_scene(scene, dark)
    lit = tmp_path / "lit.built"
    scene = Scene()
    scene._home_thumbnail_png = gradient().png()
    igz.save_scene(scene, lit)
    recent.remember(dark)
    recent.remember(lit)
    ws.show(win)
    wait_loader(win)
    sh = sheet(win)
    by_name = {r["label"]: r for k in ("cont", "recent") for r in section(sh, k)}
    assert "icon" not in by_name["dark.built"]          # placeholder
    assert by_name["lit.built"]["icon"].startswith("thumb:")


# ---- The picture a save keeps (Metal view) ------------------------------------
def saved_picture(path):
    import zipfile
    with zipfile.ZipFile(path) as archive:
        if "thumbnail.png" not in archive.namelist():
            return None
        return archive.read("thumbnail.png")


def save_with(win, monkeypatch, path, image):
    from built_render import shell_files as files

    view = win._doc.stand_in
    asked = []
    def render_pixels(width):
        asked.append(width)
        if image is None:
            return 0, 0, b""
        scaled = image.resized(width, round(image.height * width / image.width))
        return scaled.width, scaled.height, scaled.rgba()

    with monkeypatch.context() as patch:
        patch.setattr(view, "renderer", object())
        patch.setattr(view, "packed", object())
        patch.setattr(view, "render_pixels", render_pixels)
        assert files.do_save(win, path)
    assert asked == [win.SAVE_PICTURE_PX]
    return saved_picture(path)


def test_save_under_metal_keeps_the_renderers_picture(win, monkeypatch, tmp_path):
    picture = save_with(win, monkeypatch, tmp_path / "metal.built", gradient(800, 500))
    assert picture is not None
    image = SurveyRaster.decode(picture)
    assert not blank_picture(image.width, image.height, image.rgba())


def test_a_blank_metal_frame_is_not_saved(win, monkeypatch, tmp_path):
    black = solid_raster(280, 175, (0, 0, 0))
    assert save_with(win, monkeypatch, tmp_path / "black.built", black) is None
    assert save_with(win, monkeypatch, tmp_path / "none.built", None) is None


def test_an_unavailable_frame_preserves_the_previous_picture(win, monkeypatch, tmp_path):
    picture = save_with(win, monkeypatch, tmp_path / "first.built", gradient())
    assert picture is not None
    again = save_with(win, monkeypatch, tmp_path / "again.built", None)
    assert again == picture


# ---- Removing recents ----------------------------------------------------------
def test_remove_from_recents_leaves_the_file(win, monkeypatch, tmp_path):
    a, b = doc(tmp_path, "a.built"), doc(tmp_path, "b.built")
    recent.remember(a)
    recent.remember(b)                                  # Continue: b
    ws.show(win)
    sheet(win)
    act(win, "menu", "file:" + str(b.resolve()), "forget")
    assert [r["path"] for r in recent.entries()] == [str(a.resolve())]
    assert b.is_file()
    assert settings().value("home/last_file", "") == ""
    wait_loader(win)
    sh = sheet(win)
    assert section(sh, "recent")[0]["label"] == "a.built"
    # Gone for good: the legacy list does not bring it back.
    act(win, "menu", "file:" + str(a.resolve()), "forget")
    assert recent.entries() == []


def test_unavailable_paths_can_be_removed_independently(win, monkeypatch, tmp_path):
    (tmp_path / "Project").mkdir()
    ride = doc(tmp_path / "Project", "ride.igz")
    other = doc(tmp_path, "other.igz")
    old_ride = tmp_path / "old" / "ride.igz"            # gone: same name, moved
    old_site = tmp_path / "old" / "site.igz"            # gone: nothing replaces it
    for p in (old_site, old_ride, other, ride):
        recent.remember(p)
    # A missing row ABOVE the existing one stays (it is not "further up").
    stale = tmp_path / "older" / "other.igz"
    rows = recent.entries()
    rows.insert(0, {"path": str(stale), "pinned": False})
    settings().setValue("home/recent_files", json.dumps(rows))
    ws.show(win)
    wait_loader(win)
    sh = sheet(win)
    missing = section(sh, "missing")
    assert {r["id"] for r in missing} == {"file:" + str(p.resolve()) for p in (old_ride, old_site, stale)}
    assert all(r["dim"] for r in missing)
    assert {r["label"] for r in section(sh, "recent")} == {ride.name, other.name}
    row = missing[0]
    assert [m for m in row["menu"] if m[0] == "forget"] == [["forget", "Remove from recents"]]
    act(win, "menu", row["id"], "forget")
    assert len(section(sheet(win), "missing")) == 2


def test_a_missing_row_still_offers_to_locate(win, monkeypatch, tmp_path):
    from built_render.shell_files import ShellFileDialog
    found = doc(tmp_path, "found.built")
    gone = tmp_path / "away" / "lost.built"
    recent.remember(gone)
    asked = []
    monkeypatch.setattr(ShellFileDialog, "getOpenFileName", lambda *a: (asked.append(a), (str(found), ""))[1])
    ws.show(win)
    wait_loader(win)
    sh = sheet(win)
    row = section(sh, "missing")[0]
    assert row["dim"]                                   # shown, missing
    act(win, "focus", row["id"])
    blocks = detail(sheet(win))
    assert sheet(win)["footer"]["action"]["id"] == "locate"
    act(win, "open", row["id"])
    assert asked and win._doc.path.resolve() == found.resolve()


def test_native_save_picture_roundtrips_rgba_without_a_widget():
    from built_render import archive_picture_png, archive_thumbnail
    row = bytes([255, 0, 0, 128, 0, 0, 255, 255])
    assert archive_picture_png(2, 2, bytes([4, 8, 12, 255]) * 4) is None
    encoded = archive_picture_png(2, 2, row * 2)
    assert archive_thumbnail(encoded, 16) == (2, 2, row * 2)
    assert archive_thumbnail(b"broken", 16) is None


def test_link_preview_follows_moved_sources_and_stops_at_a_cycle(tmp_path):
    import json
    import zipfile

    from built_core import scan_document_links

    root = tmp_path / "host.igz"
    child = tmp_path / "moved.igz"
    stale = str(tmp_path / "old" / child.name)
    first = {"link": {"path": stale, "rel": child.name}}
    with zipfile.ZipFile(root, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("document.json", json.dumps({"groups": [first, first]}))
    child.write_text(json.dumps({"groups": [{"link": {"path": str(root)}}]}))

    assert scan_document_links(root) == [
        {"path": stale, "found": str(child.resolve()), "depth": 0, "via": str(root)},
        {"path": str(root), "found": str(root), "depth": 1, "via": str(child.resolve())},
    ]

def test_saved_summary_does_not_read_document_or_geometry(tmp_path):
    from built_core import start_document_summary
    import zipfile
    path = tmp_path / "indexed.built"
    scene = Scene()
    scene.plugin_data["padding"] = "x" * (1024 * 1024)
    igz.save_scene(scene, path)
    with zipfile.ZipFile(path) as archive:
        index = archive.read("start-summary.json")
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("start-summary.json", index)
        archive.writestr("document.json", b"unreadable model")
    assert start_document_summary(path) == {"units": "m", "links": []}


def test_switching_templates_discards_unit_edits(win):
    from built_core import units
    usual = dict(units.new_document_units())
    ws.show(win)
    sheet(win)
    act(win, "v", "units", "1")
    assert units.new_document_units() == usual
    assert win.viewport.scene.units["length"] == usual["length"]
    act(win, "focus", "tpl:steel")
    sh = sheet(win)
    field = next(b["fields"][0] for b in detail(sh) if b["kind"] == "fields")
    assert field[7][field[4]] == "mm"
    act(win, "verb", sh["footer"]["action"]["id"])
    assert win.viewport.scene.units["length"] == "mm"


def test_model_units_menu_changes_the_open_model(win):
    from built_core import units
    usual = dict(units.new_document_units())
    ws.action(win, ws.SEP.join(("units", "1")))
    assert win.viewport.scene.units["length"] == "cm"
    assert units.new_document_units() == usual


def test_empty_projects_tab_offers_manifest_actions(win):
    ws.show(win)
    sheet(win)
    act(win, "tab", "projects")
    sh = sheet(win)
    assert sh["focus"] is None
    assert sh["footer"]["action"]["id"] == sh["default_verb"] == "project"
    assert sh["verbs"][0]["label"] == "Open project…"
    assert any(".builtproject" in b.get("text", "") for b in detail(sh))
