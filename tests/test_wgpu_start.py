# SPDX-License-Identifier: GPL-3.0-or-later
"""The start page (views/wgpu_start.py): drawn by the Built chrome when it
can, the Qt Home dialog when it cannot; what it sends the chrome; what its
actions do. Offscreen there is no Metal frame: ``available`` is forced and
a stand-in renderer records what the page sends (the layout, filter and
arrow keys are tested in Rust, ui/start.rs)."""
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
from views import wgpu_start as ws

_app = QApplication.instance() or QApplication([])


class FakeRenderer:
    def __init__(self):
        self.calls = []
        self.shown = False
        self.keys = []
        self.claims = False

    def ui_show_start(self, shown, back):
        self.shown = shown
        self.calls.append(("show", shown, back))

    def ui_start_shown(self):
        return self.shown

    def ui_set_start(self, *rows):
        self.calls.append(("rows", rows))

    def ui_set_icons(self, icons):
        self.calls.append(("icons", [i[0] for i in icons]))

    def ui_start_key(self, name, text):
        self.keys.append((name, text))
        return True

    def ui_claims_key(self, name):
        return self.claims

    def rows(self):
        return [c[1] for c in self.calls if c[0] == "rows"][-1]


class FakeView:
    def __init__(self, win):
        self.renderer = FakeRenderer()
        self._ui_attached = True
        self._win = win
        self.updates = 0

    def window(self):
        return self._win

    def update(self):
        self.updates += 1

    def setFocus(self, *_):
        pass


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
    monkeypatch.setattr("views.wgpu_ui._window", lambda v: w)
    w._fake_view = view
    yield w
    for d in w.documents():
        d.saved_content = d.viewport.scene.content_version
    w._saved_version = w.viewport.scene.version
    p = w.__dict__.get("_start_page")
    if p is not None and p.dog is not None:
        p.dog.stop()
    home = getattr(w, "_home", None)
    if home is not None:
        home.close()
    w.close()


def on(monkeypatch):
    monkeypatch.setattr(ws, "available", lambda: True)


def thumb_png(w=40, h=24):
    image = QImage(w, h, QImage.Format_RGB32)
    image.fill(Qt.darkCyan)
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
    view = win._fake_view
    ws.before_frame(view, win)
    return view.renderer


def wait_loader(win):
    p = ws.page(win)
    if p.loader is not None:
        p.loader.join(5)
    p._poll()


# ---- Shown or not ------------------------------------------------------------
def test_headless_and_ui_off_keep_the_qt_dialog(win, monkeypatch):
    # Offscreen (this run): the chrome cannot draw it.
    assert not ws.available()
    win.show_home()
    assert not ws.shown(win)
    assert win._home.isVisible()
    win._home.close()
    # BUILT_UI=0 on a real screen: the same.
    monkeypatch.setattr("views.wgpu_ui.ENABLED", False)
    monkeypatch.setattr("PySide6.QtGui.QGuiApplication.platformName", staticmethod(lambda: "cocoa"))
    assert not ws.available()


def test_the_window_opens_on_it_only_without_a_file(monkeypatch):
    on(monkeypatch)
    assert ws.wants_at_start(False)
    assert not ws.wants_at_start(True)
    monkeypatch.setattr(ws, "available", lambda: False)
    assert not ws.wants_at_start(False)


def test_show_home_draws_the_page_and_sends_rows(win, monkeypatch, tmp_path):
    on(monkeypatch)
    a, b = doc(tmp_path, "alpha.built"), doc(tmp_path, "beta.built", picture=False)
    recent.remember(a)
    recent.remember(b)
    recent.note(a, {"project": "Ride", "links": 2, "imported": "skp", "unfiled": 5,
                    "problems": 13, "workspace": "Model"})
    win.show_home()
    assert ws.shown(win)
    assert getattr(win, "_home", None) is None          # no Qt dialog
    r = frame(win)
    assert ("show", True, False) in r.calls             # blank untitled behind: no return
    wait_loader(win)
    r = frame(win)
    files, projects, templates, cont, footer, nav, links = r.rows()
    names = [f[0][1] for f in files]
    assert names == ["beta.built", "alpha.built"]       # file names, not paths
    alpha = files[1][0]
    assert "Ride" in alpha[2] and "links 2 files" in alpha[2] and "from .skp" in alpha[2] \
        and "5 unfiled" in alpha[2]
    assert alpha[4].startswith("thumb:")                # its picture, uploaded once
    assert files[0][0][4] == ""                         # beta has none
    assert any(c[0] == "icons" and alpha[4] in c[1] for c in r.calls)
    # Continue: the last one opened (beta), its project line and no problems.
    assert cont[0] == str(b.resolve()) and cont[7] == "0"
    assert [t[0] for t in templates] == ["survey", "steel", "arch_mm", "blank"]
    assert footer.startswith("Engine:")
    assert [n[0] for n in nav][:3] == ["recent", "projects", "templates"]
    assert [l[0] for l in links] == ["prefs", "extensions", "help"]


def test_continue_names_problems_and_a_newer_autosave(win, monkeypatch, tmp_path):
    on(monkeypatch)
    a = doc(tmp_path, "job.built")
    recent.remember(a)
    recent.note(a, {"problems": 13, "workspace": "Model › North"})
    slot = tmp_path / "slot.built"
    slot.write_bytes(a.read_bytes())
    import os
    os.utime(slot, (a.stat().st_mtime + 60, a.stat().st_mtime + 60))
    monkeypatch.setattr("core.autosave.pending", lambda p, *k, **kw: slot)
    ws.show(win)
    wait_loader(win)
    r = frame(win)
    cont = r.rows()[3]
    # The card and its tile share one picture, sent once: two loads of a
    # key between frames crashed egui-wgpu (a texture freed mid-upload).
    sent = [k for c in r.calls if c[0] == "icons" for k in c[1]]
    assert cont[6] and sent.count(cont[6]) == 1
    assert cont[3] == "Last in Model › North"
    assert cont[4] == "13 items to fix"
    assert cont[5] == "Auto-save is newer than the file."


def test_first_frame_never_waits_for_pictures(win, monkeypatch, tmp_path):
    on(monkeypatch)
    a = doc(tmp_path, "alpha.built")
    recent.remember(a)
    gate = __import__("threading").Event()
    real = ws._load_one
    monkeypatch.setattr(ws, "_load_one", lambda p: (gate.wait(5), real(p))[1])
    ws.show(win)
    files = frame(win).rows()[0]                        # drawn before the worker is done
    assert files[0][0][4] == ""
    gate.set()
    wait_loader(win)
    assert frame(win).rows()[0][0][0][4].startswith("thumb:")


# ---- Keys --------------------------------------------------------------------
def key_event(key, text="", mods=Qt.NoModifier):
    return QKeyEvent(QEvent.KeyPress, key, mods, text)


def test_keys_go_to_the_page_but_command_keys_stay_the_windows(win, monkeypatch):
    on(monkeypatch)
    view = win._fake_view
    assert not ws.key(view, key_event(Qt.Key_Escape), True)        # not shown
    ws.show(win)
    assert ws.claims_key(view, key_event(Qt.Key_Escape))
    assert ws.key(view, key_event(Qt.Key_Escape), True)
    assert ws.key(view, key_event(Qt.Key_A, "a"), True)
    assert ws.key(view, key_event(Qt.Key_Down), True)
    assert view.renderer.keys == [("Escape", ""), ("", "a"), ("ArrowDown", "")]
    # ⌘O, ⌘N, ⌘K: the window's.
    assert not ws.claims_key(view, key_event(Qt.Key_O, "o", Qt.ControlModifier))
    assert not ws.key(view, key_event(Qt.Key_K, "k", Qt.ControlModifier), True)
    assert not ws.key(view, key_event(Qt.Key_F3), True)


def test_escape_returns_only_to_an_open_document(win, monkeypatch, tmp_path):
    on(monkeypatch)
    ws.show(win)
    ws.action(win, "close")                 # only the blank untitled behind
    assert ws.shown(win)
    a = doc(tmp_path, "open.built")
    assert win.open_path(a)                 # (opening dismisses it)
    assert not ws.shown(win)
    ws.show(win)
    r = frame(win)
    assert ("show", True, True) in r.calls
    cont = r.rows()[3]
    assert cont[7] == "1" and cont[1].startswith("open")    # «Open now»: the document behind
    ws.action(win, "close")
    assert not ws.shown(win)
    assert ("show", False, True) in frame(win).calls


# ---- Actions -----------------------------------------------------------------
def test_enter_on_a_tile_opens_it_through_the_window(win, monkeypatch, tmp_path):
    on(monkeypatch)
    a = doc(tmp_path, "tile.built")
    recent.remember(a)
    opened = []
    real = win.open_path
    monkeypatch.setattr(win, "open_path", lambda p: (opened.append(p), real(p))[1])
    ws.show(win)
    ws.action(win, "open" + ws.SEP + str(a.resolve()))
    assert opened == [a.resolve()]
    assert win._doc.path.resolve() == a.resolve()
    assert not ws.shown(win)
    assert QSettings().value("home/last_file") == str(a.resolve())


def test_a_moved_file_is_located(win, monkeypatch, tmp_path):
    on(monkeypatch)
    from views.filedialogs import file_dialogs
    moved = doc(tmp_path, "moved.built")
    old = tmp_path / "gone.built"
    recent.remember(old)
    monkeypatch.setattr(file_dialogs, "getOpenFileName", lambda *a: (str(moved), ""))
    ws.show(win)
    ws.action(win, "open" + ws.SEP + str(old.resolve()))
    assert win._doc.path.resolve() == moved.resolve()
    assert str(old.resolve()) not in [r["path"] for r in recent.entries()]


def test_pin_toggles_and_pinned_come_first(win, monkeypatch, tmp_path):
    on(monkeypatch)
    a, b = doc(tmp_path, "a.built"), doc(tmp_path, "b.built")
    recent.remember(a)
    recent.remember(b)
    ws.show(win)
    ws.action(win, "pin" + ws.SEP + str(a.resolve()))
    rows = recent.entries()
    assert rows[0] == {"path": str(a.resolve()), "pinned": True}
    files = frame(win).rows()[0]
    assert files[0][0][0] == str(a.resolve()) and files[0][1][0] == 1
    ws.action(win, "pin" + ws.SEP + str(a.resolve()))
    assert not recent.entries()[0]["pinned"] or recent.entries()[0]["path"] != str(a.resolve())
    # Pinning is not opening: Continue stays on the last document opened.
    assert QSettings().value("home/last_file") == str(b.resolve())


def test_new_model_and_templates(win, monkeypatch):
    on(monkeypatch)
    monkeypatch.setattr("views.units_start_dialog.offer_at_start", lambda w: None)
    ws.show(win)
    ws.action(win, "new")
    assert not ws.shown(win)
    ws.show(win)
    ws.action(win, "template" + ws.SEP + "steel")
    assert not ws.shown(win)
    scene = win.viewport.scene
    from core import bim_registry
    reg = bim_registry.get(scene)
    assert reg["structure"]["nodes"]["steel.beams"]["recipe"]["class"] == "IfcBeam"
    assert reg["structure"]["active"] == "steel.beams"
    assert scene.units["length"] == "mm"
    assert win._doc_is_blank()              # a template is where it starts, not an edit


@pytest.mark.parametrize("tid", [t[0] for t in ws.TEMPLATES if t[3]])
def test_every_template_structure_is_valid(tid):
    from core import bim_registry
    t = next(t for t in ws.TEMPLATES if t[0] == tid)
    scene = Scene()
    assert ws.apply_structure(scene, t[3], t[4])
    assert not bim_registry.validate(bim_registry.get(scene))


def test_a_document_coming_in_closes_the_page(win, monkeypatch, tmp_path):
    on(monkeypatch)
    ws.show(win)
    frame(win)
    a = doc(tmp_path, "dropped.built")
    win._open_path_here(a, a.suffix)         # not through the page or open_path
    frame(win)
    assert not ws.shown(win)


def test_no_frame_drawn_falls_back_to_the_qt_dialog(win, monkeypatch):
    on(monkeypatch)
    monkeypatch.setattr(ws, "_view", lambda _w: None)
    ws.show(win)
    ws.page(win)._watchdog()
    assert not ws.shown(win)
    assert win._home is not None


def test_note_remembers_what_the_window_knows(win, tmp_path):
    a = doc(tmp_path, "noted.built")
    assert win.open_path(a)
    m = recent.meta(a)
    assert m["workspace"] == "Model" and m["links"] == 0
    raw = json.loads(QSettings().value("home/meta"))
    assert str(a.resolve()) in raw
