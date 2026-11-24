# SPDX-License-Identifier: GPL-3.0-or-later
"""Forms on the Built chrome (views/chrome_forms.py): prompts as fields on
the egui sheet, answered by continuation, never by a nested loop, so the
same calls work in the ``built`` shell, which has no Qt event loop.

Offscreen there is no Metal frame: ``live`` is replaced by a stand-in
view and the test plays the chrome, sending the ``dlg:`` actions the
renderer would (rust/ingetrazo_render/src/ui/dialog.rs)."""
from __future__ import annotations

import sys
import types

import pytest
from PySide6.QtCore import QEvent, Qt
from PySide6.QtGui import QKeyEvent
from PySide6.QtWidgets import QApplication

from views import chrome_dialogs as cd
from views import chrome_forms as cf

_app = QApplication.instance() or QApplication([])


class FakeRenderer:
    def __init__(self):
        self.dialogs = []
        self.keys = []
        self.texts = []

    def ui_set_dialog(self, seq, title="", note="", width=420.0, fields=(), buttons=()):
        self.dialogs.append((seq, title, note, width, list(fields), list(buttons)))

    def ui_claims_key(self, name, mods=0):
        return True

    def ui_key(self, name, pressed, mods):
        self.keys.append((name, pressed))

    def ui_text(self, text):
        self.texts.append(text)


class FakeView:
    def __init__(self):
        self._ui_frames = 0
        self._ui_attached = True
        self.renderer = FakeRenderer()
        self.updates = 0

    def update(self):
        self.updates += 1


@pytest.fixture
def win(monkeypatch):
    from views.main_window import MainWindow
    w = MainWindow(defer_extensions=True)
    view = FakeView()
    monkeypatch.setattr(cd, "live", lambda parent: view if cd.window_of(parent) is w else None)
    monkeypatch.setattr(cd, "view_of", lambda win: view if win is w else None)
    w._fake_view = view
    yield w
    for form in list(cf.forms(w)):
        form.close()                    # releases the window's guard
    w._saved_version = w.viewport.scene.version
    w.close()


def drawn(win):
    """One frame of the chrome: the form on top is sent and marked shown."""
    view = win._fake_view
    cf.before_frame(view, win)
    return view.renderer.dialogs[-1] if view.renderer.dialogs else None


def chrome(win, *actions):
    from views import wgpu_ui
    wgpu_ui._run(win, list(actions))


def dlg(form, kind, fid, value=None):
    parts = [form.seq, kind, fid] + ([value] if value is not None else [])
    return "dlg:" + "\x1f".join(parts)


# ---- The form as data --------------------------------------------------------
def test_wire_carries_keys_defaults_and_roles():
    form = cf.Form("Export", [
        cf.Field("p", "page", "General"),
        cf.Field("name", "edit", "Name", "Caja", focus=True),
        cf.Field("n", "number", "Segments", 2, lo=2, hi=500, suffix="pcs"),
        cf.Field("w", "number", "Width", 1.5, lo=0, hi=10, decimals=3),
        cf.Field("on", "check", "Auto-save", True, row_label="Saving"),
        cf.Field("unit", "choice", "Unit", "cm", options=[("mm", "Millimetres"), ("cm", "Centimetres")]),
        cf.Field("off", "edit", "Later", "", enabled=lambda f: f.value("on") is False),
    ], buttons=[("cancel", "Cancel"), ("ok", "Export")], default="ok", cancel="cancel")
    seq, title, note, width, rows, buttons = form.wire()
    by = {r[0]: r for r in rows}
    assert title == "Export" and seq == form.seq
    assert by["p"][1] == "page" and by["p"][2] == "General"
    assert by["name"][3] == "Caja" and by["name"][8] & cf.FOCUS
    assert by["n"][3] == "2" and by["n"][5:7] == (2.0, 500.0) and by["n"][7] == ["pcs"]
    assert by["w"][3] == "1.5"
    assert by["on"][4] == 1.0 and by["on"][3] == "Saving"
    assert by["unit"][4] == 1.0 and by["unit"][7] == ["Millimetres", "Centimetres"]
    assert by["off"][8] & cf.DISABLED
    assert buttons == [("cancel", "Cancel", "esc", 3), ("ok", "Export", "↩", 1)]


def test_edits_are_typed_into_values_and_enable_rows_live():
    seen = []
    form = cf.Form("T", [cf.Field("on", "check", "On", False),
                         cf.Field("n", "number", "N", 5, lo=1, hi=10),
                         cf.Field("later", "edit", "Later", "", enabled=lambda f: f.value("on"))],
                   on_change=lambda f, fid, v: seen.append((fid, v)))
    form.receive("on", "1")
    form.receive("n", "4")
    form.receive("n", "4x")              # half typed: the value stays
    form.receive("n", "99")              # clamped to the range
    assert form.values() == {"on": True, "n": 10, "later": ""}
    assert seen == [("on", True), ("n", 4), ("n", 10)]
    rows = {r[0]: r for r in form.wire()[4]}
    assert not rows["later"][8] & cf.DISABLED


def test_validation_keeps_the_form_open_and_cancel_skips_it():
    answers = []
    form = cf.Form("T", [cf.Field("name", "edit", "Name", "")],
                   validate=lambda f, a: "A name, please." if not f.value("name") else None,
                   on_done=lambda a, v: answers.append((a, v)))
    form.press("ok")
    assert answers == [] and form.error == "A name, please."
    assert "A name, please." in form.wire()[2]
    form.press("cancel")
    assert answers == [("cancel", {"name": ""})]
    form.press("ok")                     # closed: nothing more
    assert len(answers) == 1


# ---- On the chrome -------------------------------------------------------------
def test_a_prompt_on_the_chrome_answers_later_and_holds_the_window(win):
    got = []
    cf.ask_text(win, "Make Component", "Component name:", "Component #1", done=got.append)
    assert got == []                      # nothing blocked: the call returned
    assert cd.modal(win)
    seq, title, _, _, rows, buttons = drawn(win)
    assert title == "Make Component" and rows[0][1:4] == ("edit", "Component name:", "Component #1")
    form = cf.current(win)
    chrome(win, dlg(form, "v", "text", "Caja"), dlg(form, "b", "ok"))
    assert got == ["Caja"]
    assert cf.current(win) is None and not cd.modal(win)
    assert drawn(win)[0] == ""            # the chrome takes it down


def test_escape_on_the_chrome_cancels(win):
    got = []
    cf.ask_number(win, "Divide", "Number of segments:", 2, 2, 500, done=got.append)
    form = cf.current(win)
    chrome(win, dlg(form, "b", "cancel"))
    assert got == [None]


def test_a_number_out_of_range_is_refused_on_the_chrome(win):
    got = []
    cf.ask_number(win, "Divide", "Number of segments:", 2, 2, 500, done=got.append)
    form = cf.current(win)
    chrome(win, dlg(form, "v", "value", "abc"), dlg(form, "b", "ok"))
    assert got == [] and form.error
    chrome(win, dlg(form, "v", "value", "7"), dlg(form, "b", "ok"))
    assert got == [7]


def test_clicks_for_a_form_already_gone_do_nothing(win):
    got = []
    cf.ask_text(win, "A", "a", "", done=got.append)
    old = cf.current(win)
    chrome(win, dlg(old, "b", "cancel"))
    cf.ask_text(win, "B", "b", "", done=got.append)
    chrome(win, dlg(old, "b", "ok"))     # queued for the first one
    assert got == [None] and cf.current(win) is not None


def test_keys_reach_the_form_through_the_qt_guard(win):
    from views import wgpu_ui
    view = win._fake_view
    calls = []
    original = wgpu_ui.key
    wgpu_ui.key = lambda v, e, pressed: calls.append((v, e.key(), pressed)) or True
    try:
        cf.ask_text(win, "A", "a", "", done=lambda _v: None)
        for kind in (QEvent.ShortcutOverride, QEvent.KeyPress, QEvent.KeyRelease):
            QApplication.sendEvent(win, QKeyEvent(kind, Qt.Key_A, Qt.NoModifier, "a"))
    finally:
        wgpu_ui.key = original
    assert calls == [(view, Qt.Key_A, True), (view, Qt.Key_A, False)]
    cf.current(win).close()


def test_make_component_names_through_the_chrome(win):
    from core.group import Group
    scene = win.viewport.scene
    from ingetrazo_core import Vec3 as QVector3D
    face = scene.mesh.add_face([QVector3D(*p) for p in ((0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0))])
    scene.select([face])
    win._on_make_component()
    form = cf.current(win)
    assert form is not None and form.value("text").startswith("Component")
    chrome(win, dlg(form, "v", "text", "Caja"), dlg(form, "b", "ok"))
    made = [g for g in scene.groups if isinstance(g, Group) and g.name == "Caja"]
    assert len(made) == 1 and made[0].is_component()
    assert not scene.mesh.faces


def test_divide_goes_through_the_protocol_once_answered(win, monkeypatch):
    from core.protocol import commands as protocol
    scene = win.viewport.scene
    from ingetrazo_core import Vec3 as QVector3D
    edge = scene.mesh.add_edge(QVector3D(0, 0, 0), QVector3D(4, 0, 0))
    scene.select([edge])
    calls = []
    real = protocol.divide_edges
    monkeypatch.setattr(protocol, "divide_edges",
                        lambda h, keys, n: calls.append((list(keys), n)) or real(h, keys, n))
    win._on_divide()
    assert calls == []                   # waiting for the count
    form = cf.current(win)
    chrome(win, dlg(form, "v", "value", "4"), dlg(form, "b", "ok"))
    assert calls == [([edge.key], 4)]
    assert len(scene.mesh.edges) == 4


def test_offscreen_prompts_deliver_typed_values(win, monkeypatch):
    monkeypatch.setattr(cd, "live", lambda parent: None)
    got = []
    cf.ask_text(win, "T", "L", "x", done=got.append)
    cf.current(win).receive("text", "x!")
    cf.current(win).press("ok")
    cf.ask_number(win, "T", "L", 2, 2, 9, done=got.append)
    cf.current(win).receive("value", "3")
    cf.current(win).press("ok")
    cf.ask_number(win, "T", "L", 1.5, 0, 9, 2, done=got.append)
    cf.current(win).press("cancel")
    cf.ask_choice(win, "T", "L", ["a", "b"], done=got.append)
    cf.current(win).receive("item", "1")
    cf.current(win).press("ok")
    assert got == ["x!", 3, None, "b"]


def test_offscreen_form_reports_edited_values(win, monkeypatch):
    monkeypatch.setattr(cd, "live", lambda parent: None)
    answers = []
    form = cf.Form("Soften", [cf.Field("angle", "number", "Angle", 20, lo=0, hi=180),
                              cf.Field("smooth", "check", "Smooth normals", False)],
                   on_done=lambda a, v: answers.append((a, v)))

    assert cf.show(win, form) == "chrome"
    chrome(win, dlg(form, "v", "angle", "45"), dlg(form, "v", "smooth", "1"), dlg(form, "b", "ok"))
    assert answers == [("ok", {"angle": 45, "smooth": True})]


def test_flow_reads_steps_top_to_bottom(win):
    out = []

    def steps():
        system = yield cf.request(cf.ask_choice, win, "Classification", "System", ["Uniclass", "OmniClass"])
        name = yield cf.request(cf.ask_text, win, "Classification", "Name", "")
        out.append((system, name))
    cf.flow(steps())
    cf.current(win).press("ok")
    cf.current(win).receive("text", "Brick")
    cf.current(win).press("ok")
    assert out == [("Uniclass", "Brick")]


# ---- Answers that wait without blocking -----------------------------------------
def test_ask_later_answers_through_the_sheet(win):
    got = []
    cd.ask_later(win, "Export", "Whole model, or the selection?",
                 [("cancel", "Cancel"), ("whole", "Whole model"), ("part", "Selection")], got.append,
                 default="part", cancel="cancel")
    assert got == [] and cd.modal(win)
    chrome(win, "sheet:whole")
    assert got == ["whole"] and not cd.modal(win)


def test_ask_later_without_a_host_cancels(capsys):
    got = []
    cd.ask_later(None, "Export", "Which?", [("cancel", "Cancel"), ("whole", "Whole model")],
                 got.append, default="whole", cancel="cancel")
    assert got == ["cancel"]
    assert "Export" in capsys.readouterr().err


def test_a_form_in_the_shell_with_no_chrome_cancels_instead_of_a_qt_dialog(monkeypatch):
    monkeypatch.setattr(cd, "in_shell", lambda: True)
    got = []
    cf.ask_text(None, "T", "L", "x", done=got.append)
    assert got == [None]


# ---- File pickers ------------------------------------------------------------------
def test_a_path_field_picks_through_file_dialogs(monkeypatch, tmp_path):
    """Qt's picker here; in the shell the same call is ShellFileDialog's
    native panel (app_shell.files.install swaps filedialogs.QFileDialog)."""
    from views import filedialogs
    seen = []

    class Picker:
        @staticmethod
        def getSaveFileName(parent, caption, directory, flt, *a, **k):
            seen.append((caption, flt))
            return str(tmp_path / "out.ifc"), flt
    monkeypatch.setattr(filedialogs, "QFileDialog", Picker)
    changes = []
    form = cf.Form("Export", [cf.Field("to", "path", "Save to", "", pick="save", filters="IFC4 (*.ifc)")],
                   on_change=lambda f, fid, v: changes.append(v))
    form.press("to")
    assert form.value("to") == str(tmp_path / "out.ifc")
    assert seen == [("Save to", "IFC4 (*.ifc)")] and changes == [str(tmp_path / "out.ifc")]


def test_the_library_image_picker_follows_the_host(monkeypatch, tmp_path):
    from views import filedialogs, wgpu_library

    class Picker:
        @staticmethod
        def getOpenFileName(parent, caption, directory, flt, *a, **k):
            return str(tmp_path / "wood.png"), flt
    monkeypatch.setattr(filedialogs, "QFileDialog", Picker)
    assert wgpu_library._pick_image(None, "Base image") == str(tmp_path / "wood.png")
