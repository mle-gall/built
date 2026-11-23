"""Sheets as a workspace of the Built window (views/wgpu_sheets.py), headless.

The native shell's host (app_shell/) runs the same view, chrome and input
code as the Qt app's Metal view; here it has no surface, so a recording
renderer stands in for the Metal one. The page itself is native: its
snapshot comes from the document (ingetrazo_core sheet_page_json), and
picking, moving, resizing and placing are tested in Rust (ui/page.rs);
here, that the adapter sends the page and runs the page's actions through
the sheet's history. Settings go to tmp_path; the model is synthetic
(scripts/sheets_fixture.py).
"""
import json
from pathlib import Path

import pytest
from PySide6.QtCore import QPointF, Qt
from tests.shell_isolation import restore_shell_patches  # noqa: F401  (autouse)

ROOT = Path(__file__).resolve().parents[1]


class Recorder:
    """The Renderer calls the workspace and the chrome make, recorded."""

    def __init__(self):
        self.calls = []
        self.uploads = []
        self.actions = []

    def __getattr__(self, name):
        if name.startswith("__"):
            raise AttributeError(name)

        def call(*args, **kwargs):
            self.calls.append((name, args))
            return None
        return call

    def names(self):
        return [c[0] for c in self.calls]

    def last(self, name):
        return next((a for n, a in reversed(self.calls) if n == name), None)

    # Answers the frame needs.
    def render_page(self, present=True):
        self.calls.append(("render_page", ()))
        return 1.0, None

    def render(self, present=True):
        self.calls.append(("render", ()))
        return 1.0, None

    def page_zoom(self):
        return 100.0

    def page_pointer_mm(self):
        return None

    def page_pointer(self, *a):
        self.calls.append(("page_pointer", a))
        return ""

    def page_key(self, name, mods):
        self.calls.append(("page_key", (name, mods)))
        return False

    def page(self):
        """The last page snapshot, parsed."""
        return json.loads(self.last("page_set")[0])

    def ui_take_actions(self):
        out, self.actions = self.actions, []
        return out

    def ui_repaint_after_ms(self):
        return None

    def ui_area_rect(self, name):
        return None

    def ui_pointer(self, *a):
        return False

    def ui_top_empty(self, *a):
        return False

    def ui_claims_key(self, name):
        return False

    def ui_wheel(self, *a):
        return False

    def ui_cursor_shape(self):
        return ""

    def ui_start_shown(self):
        return False


@pytest.fixture
def host(tmp_path, monkeypatch):
    from app_shell.host import Host
    from views import wgpu_sheets
    from views.composer import ComposerWindow
    # Model views would render through the Metal view, absent headless.
    monkeypatch.setattr(ComposerWindow, "render_frame", lambda self, f: None)
    monkeypatch.setattr(wgpu_sheets, "FORCE", True)
    import sys
    sys.path.insert(0, str(ROOT / "scripts"))
    import sheets_fixture
    doc = tmp_path / "sheet.igz"
    sheets_fixture.build(doc)
    h = Host(0, 1280, 800, 2.0, settings=str(tmp_path / "settings"))
    assert h.open(doc)
    from PySide6.QtCore import QTimer
    view = h.view
    view.renderer = Recorder()
    view._ui_attached = True
    view._ui_tools_sent = True
    view._ui_timer = QTimer(view, singleShot=True)
    yield h
    comp = getattr(h.window, "_composer", None)
    if comp is not None:
        comp.close()
    view.renderer = None
    h.close()


def _ws(host):
    from views import wgpu_sheets
    return wgpu_sheets.workspace_of(host.view)


def _sheets(host):
    host.window._to_sheets()
    ws = _ws(host)
    assert ws is not None
    host.view.draw_frame()
    return ws


def _act(host, arg):
    from views import wgpu_sheets
    wgpu_sheets.action(host.window, arg)
    host.view.draw_frame()


def _run_actions(host, *actions):
    """As the chrome reports them: through wgpu_ui._run."""
    from views import wgpu_ui
    wgpu_ui._run(host.window, list(actions))


# ---- switching ---------------------------------------------------------------------
def test_the_workspace_switch_swaps_the_frame_not_the_window(host):
    rec = host.view.renderer
    _run_actions(host, "tab:sheet")
    ws = _ws(host)
    assert ws is not None and ws.composer.EMBEDDED
    assert ("ui_show_sheets", (True,)) in rec.calls
    rec.calls.clear()
    host.view.draw_frame()
    assert "render_page" in rec.names() and "render" not in rec.names()
    page = rec.page()                                                # the native page
    assert page["size"] == [420.0, 297.0] and page["name"] == "Plans"
    kinds = sorted(i["kind"] for i in page["items"])
    assert kinds == ["frame", "frame", "text", "titleblock"]
    tabs = rec.last("ui_set_tabs")[0]
    assert [t[1] for t in tabs[0]] == ["Model", "Sheets"] and tabs[0][1][4] == 1
    assert tabs[1] == []                                            # no scene stepper on sheets
    assert not ws.composer.testAttribute(Qt.WA_WState_Visible)      # never shown
    _run_actions(host, "tab:model")
    assert _ws(host) is None
    assert ("ui_show_sheets", (False,)) in rec.calls and "page_clear" in rec.names()
    assert host.errors == []


def test_repeated_switches_never_break(host):
    for _ in range(30):
        host.window._to_sheets()
        host.view.draw_frame()
        host.window._show_model()
        host.view.draw_frame()
    host.window._to_sheets()
    host.view.draw_frame()
    assert _ws(host) is not None
    # Closing the composer while the workspace is on gives the frame back.
    host.window._composer.close()
    assert _ws(host) is None
    host.view.draw_frame()
    assert host.errors == []


def test_the_page_follows_the_document_and_only_then(host):
    from views.composer import TextItem
    ws = _sheets(host)
    rec = host.view.renderer
    from PySide6.QtWidgets import QApplication
    QApplication.processEvents()                 # the composer's deferred rebuilds
    host.view.draw_frame()
    rec.calls.clear()
    host.view.draw_frame()
    assert "page_set" not in rec.names()                             # nothing changed
    item = next(it for it in ws.composer.canvas.items() if isinstance(it, TextItem))
    x0 = item.model.x_mm
    item.setSelected(True)
    ws.composer.on_selection_changed()
    ws.composer.nudge_selected(5.0, 0.0)
    QApplication.processEvents()
    host.view.draw_frame()
    text = next(i for i in rec.page()["items"] if i["kind"] == "text")
    assert text["rect"][0] == pytest.approx(x0 + 5.0)
    assert rec.last("page_select")[0] == [item.model._doc_key]       # the dock's selection, on the page


def test_frames_show_the_vector_pass_and_the_renders(host, monkeypatch):
    import numpy as np
    from PySide6.QtGui import QImage
    from views.composer import ComposerWindow

    def render(self, frame):
        if frame.style == "vectorial":
            self.hlr_cache[id(frame)] = np.array([[0, 0, 10, 0], [0, 0, 0, 5]], float)
            self.hlr_kinds[id(frame)] = np.array([0, 1], np.int8)
            return None
        image = QImage(40, 30, QImage.Format_RGB32)
        image.fill(0xff336699)
        self.render_cache[id(frame)] = image
        return image
    monkeypatch.setattr(ComposerWindow, "render_frame", render)
    ws = _sheets(host)
    rec = host.view.renderer
    for _ in range(3):
        host.view.draw_frame()                                       # one frame rendered per turn
    plan, iso = ws.composer.comp.frames
    lines = rec.last("page_frame_lines")
    assert lines[0] == plan._doc_key
    assert lines[1].tolist() == [[0, 0, 10, 0], [0, 0, 0, 5]] and lines[2].tolist() == [0, 1]
    image = rec.last("page_frame_image")
    assert image[:3] == (iso._doc_key, 40, 30) and len(image[3]) == 40 * 30 * 4
    assert image[3][:4].tolist() == [0x33, 0x66, 0x99, 0xff]
    rec.calls.clear()
    host.view.draw_frame()
    assert "page_frame_lines" not in rec.names() and "page_frame_image" not in rec.names()


# ---- the sheet list ------------------------------------------------------------------
def test_sheet_list_add_duplicate_rename_reorder_delete(host):
    ws = _sheets(host)
    comps = host.viewport.scene.compositions
    first = comps[0].name
    _act(host, "add")
    assert len(comps) == 2 and ws.composer.comp is comps[1]
    _act(host, "dup\x1f0")
    assert len(comps) == 3 and comps[1].name.startswith(first)
    _act(host, "rename\x1f2\x1fRecord set")
    assert comps[2].name == "Record set"
    _act(host, "up\x1f2")
    assert comps[1].name == "Record set"
    _act(host, "down\x1f1")
    assert comps[2].name == "Record set"
    _act(host, "del\x1f2")                    # asked in the dock first
    assert len(comps) == 3
    assert host.view.renderer.last("ui_set_sheet_prompt")[0].startswith("Delete")
    _act(host, "answer\x1fdel\x1f")
    assert [c.name for c in comps] == [first, comps[1].name] and len(comps) == 2
    _act(host, "open\x1f0")
    assert ws.composer.comp is comps[0]
    rows = host.view.renderer.last("ui_set_sheet_list")[0]
    assert rows[0][0] == first and rows[0][2] is True


# ---- the inspector --------------------------------------------------------------------
def _select(ws, kind):
    from views import composer as c
    item = next(it for it in ws.composer.canvas.items() if isinstance(it, kind))
    ws.composer.canvas.clearSelection()
    item.setSelected(True)
    ws.composer.on_selection_changed()
    return item


def _row(ws, label):
    title, sections = ws.inspector()
    return next((v for s in sections for lab, v in s[1] if lab == label), None)


def test_a_viewport_s_scale_and_style_edit_through_the_dock(host):
    from views.composer import FrameItem
    ws = _sheets(host)
    item = _select(ws, FrameItem)
    title, _ = ws.inspector()
    assert title == "Model view"
    scale = _row(ws, "Scale").split("\x1d")
    options = scale[3].split("|")
    want = next(i for i, o in enumerate(options) if o.strip() == "1:20")
    fid = scale[2]
    _act(host, f"edit\x1f{fid}\x1f{want}")
    assert item.model.scale_n == pytest.approx(20.0)
    assert ws.composer.history.can_undo() if hasattr(ws.composer.history, "can_undo") else True
    width = _row(ws, "Width").split("\x1d")
    _act(host, f"edit\x1f{width[2]}\x1f123,5 mm")
    assert item.model.w_mm == pytest.approx(123.5)
    status = host.view.renderer.last("ui_set_sheet_status")
    assert status[1] == "1:20"


def test_a_text_edits_through_the_dock(host):
    from core.composition import TextoItem
    from views.composer import TextItem
    ws = _sheets(host)
    ws.composer.comp.texts.append(TextoItem(x_mm=30, y_mm=30, w_mm=60, text="Hola"))
    ws.composer._rebuild_canvas()
    item = _select(ws, TextItem)
    assert ws.inspector()[0] == "Text"
    text = _row(ws, "Text").split("\x1d")
    _act(host, f"edit\x1f{text[2]}\x1fMuro norte\nlevantado 2026")
    assert item.model.text == "Muro norte\nlevantado 2026"
    bold = _row(ws, "Bold").split("\x1d")
    _act(host, f"edit\x1f{bold[2]}\x1f1")
    assert item.model.bold is True


def test_the_sheet_itself_when_nothing_is_selected(host):
    ws = _sheets(host)
    ws.composer.canvas.clearSelection()
    ws.composer.on_selection_changed()
    title, _ = ws.inspector()
    assert title == "Sheet"
    name = _row(ws, "Name").split("\x1d")
    _act(host, f"edit\x1f{name[2]}\x1fPlanta general")
    assert ws.composer.comp.name == "Planta general"


# ---- keys, pointer, pan and zoom --------------------------------------------------------
def _texts(host, ws):
    from core.composition import TextoItem
    from views.composer import TextItem
    ws.composer.comp.texts.append(TextoItem(x_mm=40, y_mm=40, w_mm=60, text="Pozo de registro"))
    ws.composer._rebuild_canvas()
    host.view.draw_frame()
    return next(it for it in ws.composer.canvas.items() if isinstance(it, TextItem)
                and it.model.text == "Pozo de registro")


def test_keys_go_to_the_page_and_window_keys_stay_the_window_s(host):
    ws = _sheets(host)
    comp = ws.composer
    item = _texts(host, ws)
    before = len(comp.comp.texts)
    item.setSelected(True)
    comp.on_selection_changed()
    host.key(True, Qt.Key_Backspace.value, 0)
    host.key(False, Qt.Key_Backspace.value, 0)
    assert len(comp.comp.texts) == before - 1
    host.key(True, Qt.Key_Z.value, Qt.ControlModifier.value, "z")     # the composer's undo
    assert len(comp.comp.texts) == before
    # A tool letter is not the model's tool any more.
    tool = type(host.viewport.active_tool).__name__
    host.key(True, Qt.Key_L.value, 0, "l")
    assert type(host.viewport.active_tool).__name__ == tool
    from views import wgpu_sheets
    from app_shell.events import ShortcutOverride
    assert not wgpu_sheets.claims_key(host.view, ShortcutOverride(Qt.Key_S.value, Qt.ControlModifier.value, "s"))
    # ⌃Tab: back to the model.
    host.key(True, Qt.Key_Tab.value, Qt.MetaModifier.value)
    assert _ws(host) is None
    assert host.errors == []


def test_a_drag_on_the_page_is_one_step_of_the_sheet_s_history(host):
    ws = _sheets(host)
    comp = ws.composer
    plan = comp.comp.frames[0]
    key = plan._doc_key
    host.pointer("move", 400, 300, 0, 0, 0)
    assert host.view.renderer.last("page_pointer")[0] == "move"     # the chrome left it to the page
    # What ui/page.rs sends for a click on the plan, then a drag that resized it.
    _run_actions(host, f"sheets:select\x1f{key}")
    assert [it.model for it in comp.canvas.selectedItems()] == [plan]
    assert ws.inspector()[0] == "Model view"
    _run_actions(host, f"sheets:rects\x1f{key}:30:25:150:120")
    assert (plan.x_mm, plan.y_mm, plan.w_mm, plan.h_mm) == (30, 25, 150, 120)
    comp._on_undo()
    assert (plan.x_mm, plan.y_mm, plan.w_mm, plan.h_mm) == (20, 20, 190, 150)
    # Two items moved together: one step.
    text = comp.comp.texts[0]
    _run_actions(host, f"sheets:rects\x1f{key}:22:20:190:150;{text._doc_key}:22:190:180:9")
    assert plan.x_mm == 22 and text.x_mm == 22 and text.y_mm == 190
    comp._on_undo()
    assert plan.x_mm == 20 and text.x_mm == 20
    # A locked item stays where it is.
    plan.locked = True
    _run_actions(host, f"sheets:rects\x1f{key}:60:60:190:150")
    assert plan.x_mm == 20
    assert host.errors == []


def test_the_view_tool_places_a_frame(host):
    ws = _sheets(host)
    comp = ws.composer
    _run_actions(host, "sheets:tool\x1fvista")
    assert comp.tool_mode == "vista"
    host.view.draw_frame()
    assert host.view.renderer.last("ui_set_sheet_active") == ("vista",)
    before = len(comp.comp.frames)
    _run_actions(host, "sheets:place\x1f230\x1f190\x1f330\x1f260")
    assert len(comp.comp.frames) == before + 1
    f = comp.comp.frames[-1]
    assert (f.x_mm, f.y_mm, f.w_mm, f.h_mm) == (230, 190, 100, 70)
    comp._on_undo()
    assert len(comp.comp.frames) == before


def test_tools_not_native_yet_open_the_classic_editor(host, monkeypatch):
    ws = _sheets(host)
    opened = []
    monkeypatch.setattr(type(ws), "open_classic", lambda self, tool="": opened.append(tool))
    _run_actions(host, "sheets:tool\x1fcota")
    assert opened == ["cota"] and ws.composer.tool_mode != "cota"
    groups = host.view.renderer.last("ui_set_sheet_tools")[0]
    names = {t[0]: t[1] for g in groups for t in g}
    assert "classic" in names["cota"] and "classic" not in names["vista"]


def test_pan_and_zoom(host):
    from PySide6.QtCore import QPointF
    ws = _sheets(host)
    rec = host.view.renderer
    _act(host, "zoom\x1f200 %")
    assert rec.last("page_zoom_to") == (200.0,)
    host.wheel(640, 400, 0, -120, 0, -120)                          # scroll pans
    assert rec.last("page_wheel") == (640.0, 400.0, 0.0, -120.0, False)
    host.wheel(640, 400, 0, 0, 0, 120, modifiers=Qt.ControlModifier.value)
    assert rec.last("page_wheel")[4] is True                         # ⌘-wheel zooms
    host.gesture("zoom", 0.5, 640, 400)
    assert rec.last("page_zoom_by") == (640.0, 400.0, 1.5)
    _act(host, "fit")
    assert rec.last("page_fit") == (False,)
    assert ws.status()[-1] == "100%"


# ---- export ----------------------------------------------------------------------------
def test_export_pdf_from_the_workspace(host, tmp_path, monkeypatch):
    from views import filedialogs
    out = tmp_path / "sheet.pdf"
    asked = []

    def save(parent, caption="", directory="", filters="", *a, **k):
        asked.append((caption, directory))
        return str(out), filters
    monkeypatch.setattr(filedialogs.file_dialogs, "getSaveFileName", staticmethod(save))
    ws = _sheets(host)
    _act(host, "export\x1fpdf")
    assert out.read_bytes()[:4] == b"%PDF"
    assert asked and asked[0][1].endswith(".pdf")                    # next to the document
    assert Path(asked[0][1]).parent == Path(host.window._current_path).parent
    assert ws.last_export == str(out)
