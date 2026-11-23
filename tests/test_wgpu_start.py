# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise the Start provider and its actions with an offscreen chrome renderer."""
from __future__ import annotations

import json
from types import SimpleNamespace

import pytest
from PySide6.QtCore import QByteArray, QBuffer, QEvent, QIODevice, QSettings, Qt
from PySide6.QtGui import QImage, QKeyEvent
from PySide6.QtWidgets import QApplication

from core import recent
from core.scene import Scene
from formats import igz
from views import chrome_sheet as cs
from views import wgpu_start as ws

_app = QApplication.instance() or QApplication([])


class FakeRenderer:
    def __init__(self):
        self.calls = []

    def ui_set_large_sheet(self, text):
        cs.check_wire(text)       # what the chrome reads (ui/large_sheet.rs)
        self.calls.append(("sheet", json.loads(text) if text else None))

    def ui_set_icons(self, icons):
        self.calls.append(("icons", [i[0] for i in icons]))

    def ui_claims_key(self, name):
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
def win(monkeypatch, tmp_path):
    from views.main_window import MainWindow
    st = QSettings()
    for k in ("home/recent_files", "home/recent_projects", "home/last_file", "home/meta", "recent_files"):
        st.remove(k)
    ws.forget_cache()
    w = MainWindow(defer_extensions=True)
    view = FakeView(w)
    monkeypatch.setattr(ws, "_view", lambda _w: view)
    monkeypatch.setattr(cs, "_view", lambda _w: view)
    monkeypatch.setattr("views.wgpu_ui._window", lambda v: w)
    w._fake_view = view
    yield w
    for d in w.documents():
        d.saved_content = d.viewport.scene.content_version
    w._saved_version = w.viewport.scene.version
    p = w.__dict__.get("_start_page")
    if p is not None and p.dog is not None:
        p.dog.stop()
    w.close()


def thumb_png(w=40, h=24):
    # Not one flat colour: the page takes that for a blank picture.
    image = QImage(w, h, QImage.Format_RGB32)
    image.fill(Qt.darkCyan)
    for x in range(w // 2):
        for y in range(h):
            image.setPixel(x, y, 0xFFD0D8E0)
    data = QByteArray()
    buf = QBuffer(data)
    buf.open(QIODevice.WriteOnly)
    image.save(buf, "PNG")
    return bytes(data)


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
    win.show_home()
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
    win.show_home()
    assert ws.shown(win) and cs.current(win) is ws.page(win).sheet
    assert getattr(win, "_home", None) is None          # no Qt dialog
    sh = sheet(win)
    assert sh["id"] == "start" and sh["closable"] is False   # blank untitled behind: no return
    assert [v["id"] for v in sh["verbs"]] == ["new", "openfile", "project"]
    assert sh["menus"][0]["id"] == "template"
    assert [o["id"] for o in sh["menus"][0]["options"]] == ["survey", "steel", "arch_mm", "blank"]
    assert sh["search"]["placeholder"]
    wait_loader(win)
    sh = sheet(win)
    # Continue: the last one opened (beta); Recent: the rest, file names.
    cont = section(sh, "cont")
    assert [r["label"] for r in cont] == ["beta.built"] and cont[0]["id"] == "cont"
    alpha = section(sh, "recent")[0]
    assert alpha["label"] == "alpha.built" and alpha["badge"] == "13"
    assert alpha["sub"].startswith("Ride")
    assert alpha["icon"].startswith("thumb:")           # its picture, uploaded once
    assert "icon" not in cont[0]                        # beta has none
    assert any(c[0] == "icons" and alpha["icon"] in c[1] for c in frame(win).calls)
    tpl = section(sh, "templates")
    assert [t["id"] for t in tpl] == ["tpl:survey", "tpl:steel", "tpl:arch_mm", "tpl:blank"]
    assert sh["footer"]["hint"].count("Engine:") == 1
    assert [l["id"] for l in sh["footer"]["links"]] == ["libraries", "prefs", "extensions", "help"]
    # The first row is focused; its preview is on the right.
    assert sh["focus"] == "cont"


def test_the_preview_shows_project_problems_unfiled_and_where_it_was_left(win, monkeypatch, tmp_path):
    a = doc(tmp_path, "job.built")
    recent.remember(a)
    recent.note(a, {"project": "Park", "problems": 13, "workspace": "Model › North", "unfiled": 50})
    slot = tmp_path / "slot.built"
    slot.write_bytes(a.read_bytes())
    import os
    os.utime(slot, (a.stat().st_mtime + 60, a.stat().st_mtime + 60))
    monkeypatch.setattr("core.autosave.pending", lambda p, *k, **kw: slot)
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
    assert kv["Project"] == "Park" and kv["Problems"] == "13 to fix" and kv["Unfiled"] == "50"
    assert kv["Left in"].startswith("Model › North")
    assert any(b["kind"] == "text" and "Auto-save is newer" in b["text"] for b in blocks)
    assert any(b["kind"] == "buttons" and any(v["id"] == "open" and v["role"] == 1 for v in b["buttons"])
               for b in blocks)


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
    assert got == [("A.igz", 0, "ok", ""), ("C.igz", 1, "ok", ""), ("D.igz", 1, "error", "missing"),
                   ("B.igz", 0, "warn", "changed on disk")]
    assert all(r.get("fixed") for r in rows)            # read, not focused


def test_first_frame_never_waits_for_pictures(win, monkeypatch, tmp_path):
    a = doc(tmp_path, "alpha.built")
    recent.remember(a)
    gate = __import__("threading").Event()
    real = ws._load_one
    monkeypatch.setattr(ws, "_load_one", lambda p: (gate.wait(5), real(p))[1])
    ws.show(win)
    cont = section(sheet(win), "cont")                  # drawn before the worker is done
    assert "icon" not in cont[0]
    gate.set()
    wait_loader(win)
    assert section(sheet(win), "cont")[0]["icon"].startswith("thumb:")


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
    return QKeyEvent(QEvent.KeyPress, key, mods, text)


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
    cont = section(sh, "cont")[0]
    assert cont["right"] == "open now" and cont["label"].startswith("open")
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
    assert QSettings().value("home/last_file") == str(a.resolve())


def test_a_moved_file_is_located(win, monkeypatch, tmp_path):
    from views.filedialogs import file_dialogs
    moved = doc(tmp_path, "moved.built")
    old = tmp_path / "gone.built"
    recent.remember(old)
    monkeypatch.setattr(file_dialogs, "getOpenFileName", lambda *a: (str(moved), ""))
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
    assert QSettings().value("home/last_file") == str(c.resolve())


def test_new_model_and_templates(win, monkeypatch):
    monkeypatch.setattr("views.units_start_dialog.offer_at_start", lambda w: None)
    ws.show(win)
    sheet(win)
    act(win, "verb", "new")
    assert not ws.shown(win)
    ws.show(win)
    sheet(win)
    act(win, "headmenu", "template", "steel")
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
    win._open_path_here(a, a.suffix)         # not through the page or open_path
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
    raw = json.loads(QSettings().value("home/meta"))
    assert str(a.resolve()) in raw


# ---- Pictures: a black one is no picture ---------------------------------------
def png_of(image):
    data = QByteArray()
    buf = QBuffer(data)
    buf.open(QIODevice.WriteOnly)
    image.save(buf, "PNG")
    return bytes(data)


def gradient(w=280, h=175):
    from PySide6.QtGui import QColor, QPainter
    image = QImage(w, h, QImage.Format_RGB32)
    image.fill(QColor(200, 214, 230))                   # sky
    p = QPainter(image)
    p.fillRect(0, h // 2, w, h // 2, QColor(120, 128, 110))   # ground
    p.fillRect(w // 3, h // 4, w // 3, h // 2, QColor(70, 70, 80))  # the model
    p.end()
    return image


def test_blank_pictures_are_told_apart():
    black = QImage(280, 175, QImage.Format_RGB32)
    black.fill(Qt.black)
    assert ws.blank_picture(black)
    grey = QImage(280, 175, QImage.Format_RGB32)
    grey.fill(Qt.darkGray)
    assert ws.blank_picture(grey)
    # A black frame with a far speck of model in it (the composite's tile).
    speck = black.copy()
    for x in range(138, 142):
        for y in range(86, 89):
            speck.setPixel(x, y, 0xFFFFFFFF)
    assert ws.blank_picture(speck)
    assert ws.blank_picture(QImage()) and ws.blank_picture(None)
    assert not ws.blank_picture(gradient())


def test_a_black_thumbnail_shows_the_placeholder(win, monkeypatch, tmp_path):
    black = QImage(280, 175, QImage.Format_RGB32)
    black.fill(Qt.black)
    dark = tmp_path / "dark.built"
    scene = Scene()
    scene._home_thumbnail_png = png_of(black)
    igz.save_scene(scene, dark)
    lit = tmp_path / "lit.built"
    scene = Scene()
    scene._home_thumbnail_png = png_of(gradient())
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
class FakeMetalView:
    """The Metal view at its boundary: ``render_image`` is the renderer's
    offscreen frame (resident geometry, no egui chrome)."""

    def __init__(self, image):
        self.renderer = object()
        self.packed = object()
        self.image = image
        self.asked = []

    def width(self):
        return 800

    def height(self):
        return 500

    def isVisible(self):
        return True

    def render_image(self, width_px, height_px=None, overlays=False, sharp_shadows=False):
        self.asked.append((width_px, height_px, overlays))
        return self.image if self.image is None else self.image.scaled(width_px, round(width_px * 500 / 800))

    def grab(self):  # the old path: the chrome over nothing
        raise AssertionError("a save must not grab the Metal widget")


def saved_picture(path):
    import zipfile
    with zipfile.ZipFile(path) as archive:
        if "thumbnail.png" not in archive.namelist():
            return None
        image = QImage()
        image.loadFromData(archive.read("thumbnail.png"), "PNG")
        return image


def save_with(win, monkeypatch, tmp_path, name, view, gl=None):
    doc = win._doc
    monkeypatch.setattr(doc, "stand_in", view)
    # The hidden GL viewport under Metal: a black framebuffer.
    black = QImage(280, 175, QImage.Format_RGB32)
    black.fill(Qt.black)
    monkeypatch.setattr(win, "_tab_thumbnail", lambda vp: gl if gl is not None else black)
    path = tmp_path / name
    try:
        win._do_save(path)
    finally:
        monkeypatch.setattr(doc, "stand_in", None)
    return path


def test_save_under_metal_keeps_the_renderers_picture(win, monkeypatch, tmp_path):
    view = FakeMetalView(gradient(800, 500))
    path = save_with(win, monkeypatch, tmp_path, "metal.built", view)
    # One offscreen frame at thumbnail size, never the window's size, no overlays.
    assert view.asked == [(win.SAVE_PICTURE_PX, None, False)]
    picture = saved_picture(path)
    assert picture is not None and picture.width() == win.SAVE_PICTURE_PX
    assert not ws.blank_picture(picture)


def test_a_blank_metal_frame_is_not_saved(win, monkeypatch, tmp_path):
    black = QImage(800, 500, QImage.Format_RGB32)
    black.fill(Qt.black)
    path = save_with(win, monkeypatch, tmp_path, "black.built", FakeMetalView(black))
    assert saved_picture(path) is None
    # Nothing to draw with yet (no renderer frame): nothing saved either,
    # and the black GL framebuffer is not taken instead.
    view = FakeMetalView(None)
    path = save_with(win, monkeypatch, tmp_path, "none.built", view)
    assert saved_picture(path) is None


def test_the_gl_view_still_gives_its_frame(win, monkeypatch, tmp_path):
    path = save_with(win, monkeypatch, tmp_path, "gl.built", None, gl=gradient())
    picture = saved_picture(path)
    assert picture is not None and not ws.blank_picture(picture)
    # A black frame (nothing drawn yet): the document keeps the picture it had.
    path = save_with(win, monkeypatch, tmp_path, "gl-black.built", None)
    again = saved_picture(path)
    assert again is not None and again == picture


# ---- Removing recents ----------------------------------------------------------
def test_remove_from_recents_leaves_the_file(win, monkeypatch, tmp_path):
    a, b = doc(tmp_path, "a.built"), doc(tmp_path, "b.built")
    recent.remember(a)
    recent.remember(b)                                  # Continue: b
    ws.show(win)
    sheet(win)
    act(win, "menu", "cont", "forget")
    assert [r["path"] for r in recent.entries()] == [str(a.resolve())]
    assert b.is_file()
    assert QSettings().value("home/last_file", "") == ""
    wait_loader(win)
    sh = sheet(win)
    assert section(sh, "recent") == []
    assert section(sh, "cont")[0]["label"] == "a.built"     # Continue falls back
    # Gone for good: the legacy list does not bring it back.
    act(win, "menu", "cont", "forget")
    assert recent.entries() == []


def test_a_moved_file_hides_its_dead_duplicate(win, monkeypatch, tmp_path):
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
    QSettings().setValue("home/recent_files", json.dumps(rows))
    ws.show(win)
    wait_loader(win)
    sh = sheet(win)
    ids = [r["id"] for k in ("cont", "recent") for r in section(sh, k)]
    assert "file:" + str(old_ride.resolve()) not in ids
    assert "file:" + str(old_site.resolve()) in ids and "file:" + str(stale) in ids
    site = next(r for r in section(sh, "recent") if r["id"] == "file:" + str(old_site.resolve()))
    assert site["dim"] and site["sub"].startswith("Not found")
    assert [tuple(m)[0] for m in site["menu"]][0] == "locate"
    # Still on the list (hidden, not dropped) until Clear missing.
    assert str(old_ride.resolve()) in [r["path"] for r in recent.entries()]
    clear = next(b for b in sh["content"]["blocks"] if b["kind"] == "buttons")["buttons"][0]
    assert clear["id"] == "clearmissing" and "(3)" in clear["label"]
    act(win, "verb", "clearmissing")
    left = [r["path"] for r in recent.entries()]
    assert left == [str(ride.resolve()), str(other.resolve())]


def test_a_missing_row_still_offers_to_locate(win, monkeypatch, tmp_path):
    from views.filedialogs import file_dialogs
    found = doc(tmp_path, "found.built")
    gone = tmp_path / "away" / "lost.built"
    recent.remember(gone)
    asked = []
    monkeypatch.setattr(file_dialogs, "getOpenFileName", lambda *a: (asked.append(a), (str(found), ""))[1])
    ws.show(win)
    wait_loader(win)
    sh = sheet(win)
    row = section(sh, "recent")[0]
    assert row["dim"]                                   # shown, missing
    act(win, "focus", row["id"])
    blocks = detail(sheet(win))
    assert any(b["kind"] == "buttons" and b["buttons"][-1]["id"] == "locate" for b in blocks)
    act(win, "open", row["id"])
    assert asked and win._doc.path.resolve() == found.resolve()

