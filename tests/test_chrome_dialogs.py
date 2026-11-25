# SPDX-License-Identifier: GPL-3.0-or-later
"""The Built chrome's sheet and jobs (rust/ingetrazo_render/src/dialogs_host.rs) stand in for
Qt's message boxes and progress dialogs. Blocking questions cancel on a
missing frame. Notices and progress can be read from the offscreen model.

Offscreen there is no Metal frame: ``live`` is replaced by a stand-in view
and the test plays the chrome (marks the sheet drawn, clicks or types)."""
from __future__ import annotations

from types import SimpleNamespace

import pytest
from ingetrazo_core import Input as Qt
from ingetrazo_core import Vec3 as QVector3D
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_render import chrome_dialogs as cd
from core.input_codes import StandardButton


@pytest.fixture
def win(native_host, monkeypatch):
    from app_shell import modal
    w = native_host.window
    monkeypatch.setattr(cd, "live", lambda parent: w.view if parent is w else None)

    def turn(done):
        callback = vars(w).pop("_test_sheet_frame", None)
        if callback is not None:
            cd.current_sheet(w).shown = True
            callback()
        else:
            native_host.timers.run_due()
        assert done()
        return True

    monkeypatch.setattr(modal, "run_until", turn)
    return w


def _png_rgb(w, h, pixel):
    """An opaque RGB PNG of one colour (standard library only)."""
    import struct
    import zlib

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\x00" + pixel * w for _ in range(h))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def chrome(win, then):
    win._test_sheet_frame = then


def press(key, mods=Qt.NoModifier, target=None):
    from app_shell import modal
    modal.dispatch(target._host, ("key", 0, 0, 0, mods.value, key.value, ""))


def dirty(win):
    win.viewport.scene.version += 1
    assert win._is_dirty()


def test_blocking_questions_require_a_visible_view(native_host, monkeypatch):
    win = native_host.window
    assert cd.live(win) is None
    monkeypatch.setattr(native_host.view, "_ui_attached", True, raising=False)
    assert cd.live(win) is None
    assert cd.live(None) is None


@pytest.mark.parametrize("key,mods,expect", [
    (Qt.Key_Escape, Qt.NoModifier, False),       # Cancel: stay
    (Qt.Key_D, Qt.ControlModifier, True),        # ⌘D: Don't Save
])
def test_unsaved_question_answers_from_the_keys(win, key, mods, expect):
    dirty(win)
    chrome(win, lambda: press(key, mods, target=win))
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


def test_no_frame_drawn_cancels_close_and_reports_failure(win, monkeypatch):
    monkeypatch.setattr(cd, "WATCHDOG_MS", 0)
    flashed = []
    monkeypatch.setattr(win.viewport, "flash_status", flashed.append)
    dirty(win)
    assert win._confirm_discard("Close “plan”?") is False
    assert flashed == ["Built could not draw the question: Close “plan”?"]
    assert cd.current_sheet(win) is None and not cd.modal(win)


def test_the_window_is_modal_while_the_sheet_waits(win):
    from app_shell import files
    dirty(win)
    calls = []
    from unittest.mock import patch

    def probe():
        assert cd.modal(win)
        press(Qt.Key_S, Qt.ControlModifier, win)
        assert not win.close_document()
        assert not win._host.confirm_window_close()
        press(Qt.Key_Escape, target=win)

    with patch.object(files, "save", lambda w: calls.append(w)):
        chrome(win, probe)
        assert not win._confirm_discard("Close “plan”?") and not calls
        assert not cd.modal(win)
        win._host.key(True, Qt.Key_S, Qt.ControlModifier.value)
        assert calls == [win]


def test_quit_with_several_projects_asks_once(win, monkeypatch):
    first = win._doc
    dirty(win)
    second = win.new_document()
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
                       StandardButton.Yes | StandardButton.No, StandardButton.Yes) == StandardButton.Yes
    sheet = got[0]
    assert [b[1] for b in sheet.buttons] == ["No", "Yes"]
    assert sheet.cancel == str(StandardButton.No.value)
    chrome(win, lambda: press(Qt.Key_Escape, target=win))
    assert cd.question(win, "Purge", "Purge unused?") == StandardButton.No
    assert cd.critical(win, "Open failed", "No such file") == StandardButton.Ok


def test_warning_before_the_first_frame_is_kept_in_the_chrome_history(native_host, monkeypatch):
    win = native_host.window
    monkeypatch.setattr(cd, "in_shell", lambda: True)
    assert cd.live(win) is None
    assert cd.warning(win, "Linked files", "Missing") == StandardButton.Ok
    row = cd.history(win)[0]
    assert (row.title, row.detail, row.tone) == ("Linked files", "Missing", "warning")


@pytest.mark.parametrize("method,title", [
    ("_on_recover_discarded", "Recover a discarded auto-save…"),
    ("_on_insert_faceme_image", "Face-me image"),
    ("_on_get_models", "Get more models and textures"),
    ("_import_rfa_path", "Revit family import report"),
    ("_on_clear_texture_cache", "Texture cache"),
])
def test_information_call_sites_post_toasts(win, tmp_path, monkeypatch, method, title):
    from core import autosave, texture
    from formats import rfa
    from views.filedialogs import file_dialogs
    args = ()
    if method == "_on_recover_discarded":
        monkeypatch.setattr(autosave, "discarded_dir", lambda: tmp_path)
    elif method == "_on_insert_faceme_image":
        picture = tmp_path / "opaque.png"
        picture.write_bytes(_png_rgb(2, 2, b"\xff\xff\xff"))
        monkeypatch.setattr(file_dialogs, "getOpenFileName", lambda *a: (str(picture), ""))

    elif method == "_import_rfa_path":
        monkeypatch.setattr(rfa, "read_rfa", lambda path: {})
        monkeypatch.setattr(rfa, "apply_payload", lambda scene, payload: ["Imported one family"])
        args = (tmp_path / "fixture.rfa",)
    elif method == "_on_clear_texture_cache":
        monkeypatch.setattr(texture, "texture_cache_stats", lambda: (0, 0))
    getattr(win, method)(*args)
    assert cd.history(win)[0].title == title
    if method == "_on_insert_faceme_image":
        # The opacity notice leaves the height question available.
        from ingetrazo_render import chrome_forms
        form = chrome_forms.current(win)
        assert form.title == title
        form.press("cancel")
    assert not cd.current_sheet(win) and not cd.modal(win)


def test_empty_ifc_export_explains_how_to_include_geometry(win, tmp_path):
    from ingetrazo_render import delivery_export_form

    path = tmp_path / "model.ifc"
    path.write_bytes(b"previous delivery")
    form = delivery_export_form(win, one_off=True, initial_format="ifc")
    form.set("destination", str(path))
    form.press("export")
    assert form.error == ("Nothing to export: tag geometry in the BIM panel first "
                          "(only tagged objects go to IFC).")
    assert not form.closed and path.read_bytes() == b"previous delivery"
    assert not cd.jobs(win)
    form.press("cancel")


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
    second = win.new_document()
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
    from core.scene import Scene
    from formats.igz import save_scene
    scene = Scene()
    for i in range(4):
        m = Mesh()
        x = i * 3.0
        m.add_face([QVector3D(x, 0, 0), QVector3D(x + 2, 0, 0),
                    QVector3D(x + 2, 2, 0), QVector3D(x, 2, 0)])
        scene.groups.append(Group(m, name=f"B{i}"))
    scene.version += 1
    path = tmp_path / "plan.igz"
    save_scene(scene, path)
    return path


def test_cancel_while_opening_stops_the_load_and_keeps_the_window(win, plan, monkeypatch):
    from formats import igz
    milestones = []

    def fake_load(scene, path, progress=None, **_k):
        for i in range(10):
            scene.version += 1
            milestones.append(i)
            progress(i / 10, "Reading geometry…")
            if i == 2:
                assert cd.jobs(win)[0].cancellable
                cd.jobs(win)[0].cancel()            # × in the top bar
    monkeypatch.setattr(igz, "load_into", fake_load)
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
