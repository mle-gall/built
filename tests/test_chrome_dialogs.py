# SPDX-License-Identifier: GPL-3.0-or-later
"""The Built chrome's sheet and jobs (views/chrome_dialogs.py) stand in for
Qt's message boxes and progress dialogs when the egui UI is on screen, and
answer synchronously as QMessageBox.exec() did; with the UI off, headless,
or no frame to draw them, Qt asks as before.

Offscreen there is no Metal frame: ``live`` is replaced by a stand-in view
and the test plays the chrome (marks the sheet drawn, clicks or types)."""
from __future__ import annotations

from types import SimpleNamespace

import pytest
from PySide6.QtCore import QEvent, Qt, QTimer
from PySide6.QtGui import QKeyEvent, QVector3D
from PySide6.QtWidgets import QApplication, QMessageBox

from views import chrome_dialogs as cd

_app = QApplication.instance() or QApplication([])


class FakeView:
    """What the chrome needs of the Metal view: frames and repaints."""

    def __init__(self):
        self._ui_frames = 0
        self.updates = 0

    def update(self):
        self.updates += 1


@pytest.fixture
def win(monkeypatch):
    from views.main_window import MainWindow
    w = MainWindow(defer_extensions=True)
    view = FakeView()
    monkeypatch.setattr(cd, "live", lambda parent: view if cd.window_of(parent) is w else None)
    w._fake_view = view
    yield w
    w._saved_version = w.viewport.scene.version
    w.close()


def chrome(win, then, *, drawn=True, ms=30):
    """Play the chrome: after ``ms`` the sheet is drawn, then ``then()``."""
    def go():
        sheet = cd.current_sheet(win)
        if sheet is not None and drawn:
            sheet.shown = True
        then()
    QTimer.singleShot(ms, go)


def press(key, mods=Qt.NoModifier, target=None):
    target = target or QApplication.instance()
    for kind in (QEvent.ShortcutOverride, QEvent.KeyPress, QEvent.KeyRelease):
        QApplication.sendEvent(target, QKeyEvent(kind, key, mods))


def dirty(win):
    win.viewport.scene.version += 1
    assert win._is_dirty()


def test_live_is_off_headless_and_with_the_ui_off(monkeypatch):
    from views.main_window import MainWindow
    from views import wgpu_ui
    w = MainWindow(defer_extensions=True)
    try:
        assert cd.live(w) is None            # offscreen: Qt asks
        monkeypatch.setattr(cd.QGuiApplication, "platformName", staticmethod(lambda: "cocoa"))
        monkeypatch.setattr(wgpu_ui, "ENABLED", False)
        assert cd.live(w) is None            # BUILT_UI=0
        assert cd.live(None) is None
    finally:
        w.close()


@pytest.mark.parametrize("key,mods,expect", [
    (Qt.Key_Escape, Qt.NoModifier, False),       # Cancel: stay
    (Qt.Key_D, Qt.ControlModifier, True),        # ⌘D: Don't Save
])
def test_unsaved_question_answers_from_the_keys(win, key, mods, expect):
    dirty(win)
    chrome(win, lambda: press(key, mods, win))
    assert win._confirm_discard("Close “plan”?") is expect
    assert cd.current_sheet(win) is None
    assert not cd.modal(win)


def test_enter_saves(win, monkeypatch):
    dirty(win)
    saved = []

    def fake_save():
        saved.append(True)
        win._saved_version = win.viewport.scene.version
    monkeypatch.setattr(win, "_on_save", fake_save)
    chrome(win, lambda: press(Qt.Key_Return, target=win))
    assert win._confirm_discard("Quit Built?") is True
    assert saved == [True]


def test_a_click_on_the_sheet_answers_through_the_chrome_actions(win):
    from views import wgpu_ui
    dirty(win)
    sheet_seen = []

    def click():
        sheet = cd.current_sheet(win)
        sheet_seen.append((sheet.title, [b[:2] for b in sheet.buttons], sheet.default,
                           sheet.cancel, sheet.discard))
        wgpu_ui._run(win, ["sheet:discard"])
    chrome(win, click)
    assert win._confirm_discard("Close “plan”?") is True
    title, buttons, default, cancel, discard = sheet_seen[0]
    assert title == "Close “plan”?"
    assert [b[0] for b in buttons] == ["discard", "cancel", "save"]
    assert (default, cancel, discard) == ("save", "cancel", "discard")


def test_no_frame_drawn_falls_back_to_qt(win, monkeypatch):
    """The watchdog: a sheet nobody saw drawn goes to Qt, never a hang."""
    monkeypatch.setattr(cd, "WATCHDOG_MS", 50)
    asked = []

    def fake_exec(box):
        asked.append(box.text())
        return QMessageBox.Cancel
    monkeypatch.setattr(QMessageBox, "exec", fake_exec)
    dirty(win)
    assert win._confirm_discard("Close “plan”?") is False
    assert asked and "Close “plan”?" in asked[0]
    assert cd.current_sheet(win) is None and not cd.modal(win)


def test_the_window_is_modal_while_the_sheet_waits(win):
    """Menus greyed, shortcuts and closes refused; all back after."""
    dirty(win)
    menus = [a for a in win.menuBar().actions() if a.isEnabled()]
    seen = {}

    def probe():
        seen["menus"] = [a.isEnabled() for a in menus]
        ev = QKeyEvent(QEvent.ShortcutOverride, Qt.Key_S, Qt.ControlModifier)
        ev.ignore()
        QApplication.sendEvent(win, ev)
        seen["shortcut_taken"] = ev.isAccepted()
        win.close()
        seen["visible_after_close"] = not win._closed if hasattr(win, "_closed") else True
        press(Qt.Key_Escape, target=win)
    chrome(win, probe)
    assert win._confirm_discard("Close “plan”?") is False
    assert seen["menus"] and not any(seen["menus"])
    assert seen["shortcut_taken"]
    assert not getattr(win, "_closed", False)
    assert all(a.isEnabled() for a in menus)


def test_quit_with_several_projects_asks_once(win, monkeypatch):
    first = win._doc
    dirty(win)
    second = win._new_document()
    win.viewport.scene.version += 1
    texts = []

    def answer():
        texts.append(cd.current_sheet(win).checklist)
        press(Qt.Key_D, Qt.ControlModifier, win)
    chrome(win, answer)
    assert win._confirm_quit() is True
    assert texts == [[(first.display_name(), True), (second.display_name(), True)]]
    for doc in (first, second):
        win.activate_document(doc)
        win._saved_version = win.viewport.scene.version


def test_message_boxes_map_standard_buttons(win):
    got = []
    chrome(win, lambda: (got.append(cd.current_sheet(win)), press(Qt.Key_Return, target=win)))
    assert cd.question(win, "Recovered drawing", "Recover it?",
                       QMessageBox.Yes | QMessageBox.No, QMessageBox.Yes) == QMessageBox.Yes
    sheet = got[0]
    assert [b[1] for b in sheet.buttons] == ["No", "Yes"]
    assert sheet.cancel == str(QMessageBox.No.value)
    chrome(win, lambda: press(Qt.Key_Escape, target=win))
    assert cd.question(win, "Purge", "Purge unused?") == QMessageBox.No
    assert cd.critical(win, "Open failed", "No such file") == QMessageBox.Ok


def test_message_boxes_go_to_qt_when_the_chrome_is_off(monkeypatch):
    from views.main_window import MainWindow
    calls = []
    monkeypatch.setattr(QMessageBox, "warning", lambda *a: calls.append(a) or QMessageBox.Ok)
    w = MainWindow(defer_extensions=True)
    try:
        assert cd.warning(w, "Linked files", "Missing") == QMessageBox.Ok
        assert calls and calls[0][1:] == ("Linked files", "Missing")
    finally:
        w.close()


@pytest.mark.parametrize("method,title", [
    ("_on_set_language", "Language changed"),
    ("_on_recover_discarded", "Recover a discarded auto-save…"),
    ("_on_insert_faceme_image", "Face-me image"),
    ("_on_get_models", "Get more models and textures"),
    ("_import_rfa_path", "Revit family import report"),
    ("_on_clear_texture_cache", "Texture cache"),
    ("_on_export_ifc", "Export IFC"),
])
def test_information_call_sites_post_toasts(win, tmp_path, monkeypatch, method, title):
    from PySide6.QtGui import QImage
    from core import autosave, texture
    from formats import rfa
    from views import main_window as mw
    args = ()
    if method == "_on_set_language":
        monkeypatch.setattr(mw, "set_language", lambda lang: None)
        monkeypatch.setattr(mw, "current_language", lambda: "en")
        args = ("fr",)
    elif method == "_on_recover_discarded":
        monkeypatch.setattr(autosave, "discarded_dir", lambda: tmp_path)
    elif method == "_on_insert_faceme_image":
        picture = tmp_path / "opaque.png"
        img = QImage(2, 2, QImage.Format_RGB32)
        img.fill(Qt.white)
        assert img.save(str(picture))
        monkeypatch.setattr(mw.file_dialogs, "getOpenFileName", lambda *a: (str(picture), ""))
        monkeypatch.setattr(mw._forms, "ask_number", lambda *a, **kw: None)
    elif method == "_import_rfa_path":
        monkeypatch.setattr(rfa, "read_rfa", lambda path: {})
        monkeypatch.setattr(rfa, "apply_payload", lambda scene, payload: ["Imported one family"])
        args = (tmp_path / "fixture.rfa",)
    elif method == "_on_clear_texture_cache":
        monkeypatch.setattr(texture, "texture_cache_stats", lambda: (0, 0))
    getattr(win, method)(*args)
    assert cd.history(win)[0].title == title
    assert not cd.current_sheet(win) and not cd.modal(win)


def test_notices_stack_expire_and_keep_actions(win, monkeypatch):
    from views import wgpu_ui
    clock = [10.]
    monkeypatch.setattr(cd.time, "monotonic", lambda: clock[0])
    seen = []
    cd.notify(win, "First", "details", action_label="Show", action=lambda: seen.append(1))
    cd.notify(win, "Second", "details")
    clock[0] += 6
    second, first = cd.history(win)
    assert [second.title, first.title] == ["Second", "First"]
    wgpu_ui._run(win, ["activity:" + first.id])
    assert seen == [1]
    clock[0] += 45
    assert not cd.history(win)


def test_save_failed_offers_recovery(win):
    seen = []

    def answer():
        sheet = cd.current_sheet(win)
        seen.append((sheet.tone, sheet.default, sheet.text))
        cd.answer(win, "retry")

    chrome(win, answer)
    cd.save_failed(win, OSError("Volume full"), lambda: seen.append("retried"))
    assert seen == [("error", "retry", "Volume full"), "retried"]


def test_quit_saves_only_checked_projects(win, monkeypatch):
    first = win._doc
    dirty(win)
    second = win._new_document()
    dirty(win)
    received = []
    monkeypatch.setattr(win, "_save_all_or_stay", lambda docs, answer: received.append((docs, answer)))
    def select():
        cd.answer(win, "check:0")
        cd.answer(win, "save")
    chrome(win, select)
    win._confirm_quit()
    assert received == [([second], "save")]
    for doc in (first, second):
        win.activate_document(doc)
        win._saved_version = win.viewport.scene.version


def test_progress_is_a_job_in_the_bar(win):
    bar, cb = win._import_progress("Opening plan.igz…", monotonic=True)
    assert isinstance(bar, cd.ChromeBar)
    assert cd.exclusive(win) and cd.modal(win)
    cb(0.42, "Reading geometry…")
    cb(0.30, "Reading geometry…")                   # monotonic: never back
    job = cd.jobs(win)[0]
    assert (job.label, job.phase, round(job.fraction, 2)) == ("Opening plan.igz…", "Reading geometry…", 0.42)
    assert not job.cancellable
    cancelled = []
    bar.canceled.connect(lambda: cancelled.append(True))
    bar.set_cancellable(True)
    press(Qt.Key_Escape, target=win)                 # Esc cancels the job
    assert cancelled == [True] and bar.wasCanceled()
    bar.close()
    assert not cd.jobs(win) and not cd.modal(win)


def test_the_jobs_cancel_comes_from_the_chrome(win):
    from views import wgpu_ui
    bar, _cb = win._import_progress("Importing x.dwg…")
    bar.setRange(0, 0)
    bar.setCancelButtonText("Cancel")
    hits = []
    bar.canceled.connect(lambda: hits.append(True))
    job = cd.jobs(win)[0]
    assert job.fraction < 0 and job.cancellable
    wgpu_ui._run(win, [f"job:cancel:{job.id}"])
    assert hits == [True]
    bar.close()


def test_busy_shows_a_job_only_while_it_runs(win):
    with cd.busy(win, "Saving plan.built…") as job:
        assert job is not None and cd.jobs(win) == [job]
        assert not cd.exclusive(win)
    assert cd.jobs(win) == []


def test_the_background_autosave_shows_as_a_quiet_job(win):
    win._background_autosave = SimpleNamespace(busy=True, job={"doc": win._doc})
    jobs = cd.jobs(win)
    assert [j.id for j in jobs] == ["autosave"] and "Auto-saving" in jobs[0].label
    assert not jobs[0].cancellable
    win._background_autosave = SimpleNamespace(busy=False, job=None)
    assert cd.jobs(win) == []


@pytest.fixture
def plan(tmp_path):
    from core.group import Group
    from core.mesh import Mesh
    from views.main_window import MainWindow
    w0 = MainWindow(defer_extensions=True)
    scene = w0.viewport.scene
    for i in range(4):
        m = Mesh()
        x = i * 3.0
        m.add_face([QVector3D(x, 0, 0), QVector3D(x + 2, 0, 0),
                    QVector3D(x + 2, 2, 0), QVector3D(x, 2, 0)])
        scene.groups.append(Group(m, name=f"B{i}"))
    scene.version += 1
    path = tmp_path / "plan.igz"
    w0._do_save(path)
    w0._saved_version = scene.version
    w0.close()
    return path


def test_cancel_while_opening_stops_the_load_and_keeps_the_window(win, plan, monkeypatch):
    import views.main_window as mw
    milestones = []

    def fake_load(scene, path, progress=None, **_k):
        for i in range(10):
            scene.version += 1
            milestones.append(i)
            progress(i / 10, "Reading geometry…")
            if i == 2:
                bar = win._progress_dlg
                assert isinstance(bar, cd.ChromeBar) and bar.job.cancellable
                cd.jobs(win)[0].cancel()            # × in the top bar
    monkeypatch.setattr(mw.igz_format, "load_into", fake_load)
    assert win.open_path(plan) is False
    assert milestones == [0, 1, 2, 3]               # unwound at the next one
    assert not getattr(win, "_closed", False)
    assert win._current_path is None and not win._is_dirty()
    assert not cd.jobs(win) and not cd.modal(win)
    assert not win._open_in_progress and not win._open_cancelled


def test_open_with_missing_sources_has_one_banner_and_problem_rows(win, tmp_path, monkeypatch):
    from core import links
    from formats import igz
    from tests.test_links import _source, _host_with_link
    from views import wgpu_ui
    from views.filedialogs import file_dialogs

    a, b = (_source(tmp_path / name) for name in ("A.igz", "B.igz"))
    host, _ = _host_with_link(tmp_path, a)
    host.groups.append(links.make_link(host, a))
    host.groups.append(links.make_link(host, b))
    path = tmp_path / "Host.built"
    igz.save_scene(host, path)
    moved = a.rename(tmp_path / "Relocated.igz")
    b.unlink()
    monkeypatch.setattr(cd, "ask", lambda *a, **kw: pytest.fail("blocking link question"))
    assert win.open_path(path)
    win._after_links_loaded()
    assert win._doc._missing_link_banner == "2 linked files missing"
    scene = win.viewport.scene
    rules = wgpu_ui._problem_rules(scene, {}, {}, {}, [])
    affected = next(groups for _, key, _, groups in rules if key == "links")
    assert len(affected) == 3
    monkeypatch.setattr(file_dialogs, "getOpenFileName", lambda *args: (str(moved), ""))
    monkeypatch.setattr(cd, "view_of", lambda _: win._fake_view)
    wgpu_ui._run(win, ["linkbanner:locate"])
    assert win._doc._missing_link_banner == "1 linked file missing"
    assert len(links.linked_groups(scene, moved)) == 2
    assert links.link_state(scene, moved) == "resolved"
    import os
    stamp = moved.stat().st_mtime_ns + 1_000_000_000
    os.utime(moved, ns=(stamp, stamp))
    assert links.link_state(scene, moved) == "modified"
    wgpu_ui._run(win, ["linkbanner:close"])
    assert win._doc._missing_link_banner == ""


def test_open_preserves_recovery_as_a_separate_copy(win, tmp_path, monkeypatch):
    from core import autosave
    from core.scene import Scene
    from formats import igz
    from views import wgpu_ui
    disk = tmp_path / "original.built"
    scene = Scene()
    igz.save_scene(scene, disk)
    scene.mesh.add_face([QVector3D(0, 0, 0), QVector3D(1, 0, 0), QVector3D(1, 1, 0)])
    autosave.write(scene, disk)
    monkeypatch.setattr(cd, "question", lambda *a, **kw: pytest.fail("blocked opening"))
    assert win.open_path(disk)
    assert not win.viewport.scene.mesh.faces and not win._is_dirty()
    recovery = next(r for r in cd.history(win) if r.action_label == "Open copy")
    wgpu_ui._run(win, ["activity:" + recovery.id])
    assert win._current_path is None and win._is_dirty()
    assert len(win.viewport.scene.mesh.faces) == 1


def test_failed_job_and_export_folder_are_retained(win, tmp_path, monkeypatch):
    from core import desktop_open
    opened = []
    monkeypatch.setattr(desktop_open, "open_url", opened.append)
    with pytest.raises(OSError), cd.busy(win, "Saving"):
        raise OSError("Volume full")
    row = cd.history(win)[0]
    assert row.tone == "error" and row.detail == "Volume full"
    with cd.busy(win, "Exporting", output_path=tmp_path / "a.pdf"):
        pass
    row = cd.history(win)[0]
    assert row.action_label == "Open folder"
    row.action()
    assert opened == [tmp_path.resolve()]
