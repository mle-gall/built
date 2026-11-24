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

    # What the page holds per frame: lines, a picture or a GPU render.
    def page_frame_ops(self, key, *a):
        self.calls.append(("page_frame_ops", (key, *a)))
        self.__dict__.setdefault("held", set()).add(key)

    def page_frame_image(self, key, *a):
        self.calls.append(("page_frame_image", (key, *a)))
        self.__dict__.setdefault("held", set()).add(key)

    def page_frame_render(self, key, *a, **kw):
        self.calls.append(("page_frame_render", (key, *a, kw)))
        self.__dict__.setdefault("held", set()).add(key)

    def page_frame_forget(self, key):
        self.calls.append(("page_frame_forget", (key,)))
        self.__dict__.setdefault("held", set()).discard(key)

    def page_has_content(self, key):
        return key in self.__dict__.get("held", ())

    def page_pointer_mm(self):
        return None

    def page_pointer(self, *a):
        self.calls.append(("page_pointer", a))
        return ""

    def page_key(self, name, mods):
        self.calls.append(("page_key", (name, mods)))
        return False

    def page_select(self, keys):
        self.calls.append(("page_select", (list(keys),)))
        self.selected = list(keys)

    def page_selection(self):
        return list(self.__dict__.get("selected", []))

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
    ws = _sheets(host)
    rec = host.view.renderer
    from PySide6.QtWidgets import QApplication
    QApplication.processEvents()                 # the composer's deferred rebuilds
    host.view.draw_frame()
    rec.calls.clear()
    host.view.draw_frame()
    assert "page_set" not in rec.names()                             # nothing changed
    text = ws.composer.comp.texts[0]
    x0 = text.x_mm
    _run_actions(host, f"sheets:set\x1f{text._doc_key}\x1f" + '{"x_mm": %r}' % (x0 + 5.0))
    host.view.draw_frame()
    item = next(i for i in rec.page()["items"] if i["kind"] == "text")
    assert item["rect"][0] == pytest.approx(x0 + 5.0)
    assert item["fields"]["text"] == text.text                      # the inspector reads the record


def test_frames_show_the_vector_pass_and_the_renders(host, monkeypatch):
    import numpy as np
    from views import wgpu_sheet_frames
    from views.composer import ComposerWindow

    def render(self, frame):
        raise AssertionError("the composer draws no frame for the page")
    monkeypatch.setattr(ComposerWindow, "render_frame", render)

    def drawing(source, frame):
        assert frame.style == "vectorial"
        return {"segs": np.array([[0, 0, 10, 0], [0, 0, 0, 5]], float), "kinds": np.array([0, 1], np.int8),
                "fills": []}
    monkeypatch.setattr(wgpu_sheet_frames, "drawing", drawing)
    shaded = []

    def native(view, frame, key):
        shaded.append(frame)
        view.renderer.page_frame_render(key)
        return True
    monkeypatch.setattr(wgpu_sheet_frames, "render", native)
    ws = _sheets(host)
    rec = host.view.renderer
    for _ in range(3):
        host.view.draw_frame()                                       # one frame rendered per turn
    plan, iso = ws.composer.comp.frames
    key, ops = rec.last("page_frame_ops")
    assert key == plan._doc_key
    lines = [o["pts"] for o in json.loads(ops) if o["op"] == "line"]
    assert lines == [[0, 0, 10, 0], [0, 0, 0, 5]]                     # edge, then profile
    assert shaded == [iso] and rec.last("page_frame_render")[0] == iso._doc_key
    assert "page_frame_image" not in rec.names()                    # no read-back, no upload
    rec.calls.clear()
    host.view.draw_frame()
    assert "page_frame_ops" not in rec.names() and "page_frame_render" not in rec.names()
    _run_actions(host, f"sheets:refresh\x1f{iso._doc_key}")         # Update view: rendered again
    host.view.draw_frame()
    assert rec.last("page_frame_forget") == (iso._doc_key,) and shaded == [iso, iso]
    iso.scale_n = 200.0                      # its window changed outside the page (an undo)
    host.view.draw_frame()
    assert shaded == [iso, iso, iso]
    iso.x_mm += 5.0                          # moved only: the picture stays
    host.view.draw_frame()
    assert shaded == [iso, iso, iso]



def test_a_shaded_frame_renders_into_the_page_without_touching_the_model(host):
    """The frame's camera and rows come from Rust; the live camera, the
    tags and the style stay as they are; the picture goes to the page."""
    import numpy as np
    import ingetrazo_core as NATIVE
    from core.composition import RENDER_DPI
    ws = _sheets(host)
    rec = host.view.renderer
    plan, iso = ws.composer.comp.frames
    vp = host.view.source_viewport
    scene = vp.scene
    cam = vp.camera
    scene.layers[0].visible = False if len(scene.layers) > 1 else True
    before = (cam.target.toTuple(), cam.distance, cam.yaw, cam.pitch, cam.perspective,
              [ly.visible for ly in scene.layers], scene.display_style, scene.folder_visibility)
    for _ in range(3):
        host.view.draw_frame()
    call = rec.last("page_frame_render")
    assert call is not None and call[0] == iso._doc_key
    assert call[1:3] == iso.render_px(RENDER_DPI)
    view = next(v for v in scene.saved_views if v.name == "Iso")
    lo, hi = scene.bounds()
    want = NATIVE.sheet_frame_camera(iso, view, None, (lo.toTuple(), hi.toTuple()))
    assert list(call[3]) == pytest.approx(want["view"], abs=1e-9)
    rows = np.frombuffer(bytes(call[6]), np.uint8).reshape(-1, 80)
    hidden = NATIVE.sheet_frame_hidden(scene._root, list(scene.groups), list(host.view.packed.groups), view)
    assert (rows[:, 72] & 1).tolist() == list(hidden)
    assert (cam.target.toTuple(), cam.distance, cam.yaw, cam.pitch, cam.perspective,
            [ly.visible for ly in scene.layers], scene.display_style, scene.folder_visibility) == before


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
    # Every verb is a step of the document's history.
    ws._undo()
    assert len(comps) == 3 and comps[2].name == "Record set"         # the deletion undone
    ws._undo()                                                        # down
    ws._undo()                                                        # up
    assert comps[2].name == "Record set"
    ws._undo()                                                        # the rename
    assert "Record set" not in [c.name for c in comps]
    ws._undo()                                                        # the duplicate
    ws._undo()                                                        # the new sheet
    assert [c.name for c in comps] == [first]
    host.view.draw_frame()
    assert ws.composer.comp is comps[0]                               # the open sheet came back
    ws._undo(redo=True)
    assert len(comps) == 2
    _act(host, "open\x1f0")
    assert ws.composer.comp is comps[0]
    rows = host.view.renderer.last("ui_set_sheet_list")[0]
    assert rows[0][0] == first and rows[0][2] is True


# ---- the inspector (ingetrazo_render sheet_inspector.rs) --------------------------------
def _set(host, model, **changes):
    import json
    _run_actions(host, f"sheets:set\x1f{model._doc_key}\x1f{json.dumps(changes)}")


def test_inspector_edits_are_steps_of_the_document_history(host):
    ws = _sheets(host)
    plan = ws.composer.comp.frames[0]
    host.view.draw_frame()
    page = host.view.renderer.page()
    assert [v[0] for v in page["choices"]["views"]][:2] == ["__current__", "std:top"]
    assert "scene:Iso" in [v[0] for v in page["choices"]["views"]]
    _set(host, plan, scale_n=20.0)
    assert plan.scale_n == pytest.approx(20.0)
    _set(host, plan, view_key="scene:Iso", cam_yaw=None, cam_target=None)
    assert plan.view_key == "scene:Iso"
    ws._undo()
    assert plan.view_key == "std:top"
    ws._undo()
    assert plan.scale_n == pytest.approx(100.0)
    ws._undo(redo=True)
    assert plan.scale_n == pytest.approx(20.0)
    assert host.errors == []


def test_the_sheet_and_several_items_through_the_inspector(host):
    import json
    ws = _sheets(host)
    comp = ws.composer.comp
    _run_actions(host, "sheets:sheet\x1f" + json.dumps({"name": "Planta general", "paper": "A2"}))
    assert (comp.name, comp.paper) == ("Planta general", "A2")
    plan, iso = comp.frames
    _run_actions(host, f"sheets:set_many\x1f{plan._doc_key}\x1f" + '{"locked": true}'
                 + f"\x1f{iso._doc_key}\x1f" + '{"locked": true}')
    assert plan.locked and iso.locked
    ws._undo()
    assert not plan.locked and not iso.locked
    ws._undo()
    assert comp.name == "Plans"


def test_delete_duplicate_and_lock_on_the_selection(host):
    ws = _sheets(host)
    comp = ws.composer.comp
    text = comp.texts[0]
    host.view.renderer.page_select([text._doc_key])
    host.key(True, Qt.Key_D.value, Qt.ControlModifier.value, "d")
    assert len(comp.texts) == 2
    copy = comp.texts[1]
    assert (copy.x_mm, copy.y_mm, copy.text) == (text.x_mm + 5.0, text.y_mm + 5.0, text.text)
    assert host.view.renderer.page_selection() == [copy._doc_key]   # the copy is the selection
    host.key(True, Qt.Key_L.value, Qt.ControlModifier.value, "l")
    assert copy.locked
    host.key(True, Qt.Key_Backspace.value, 0)
    assert len(comp.texts) == 2                                       # a locked item stays
    host.key(True, Qt.Key_L.value, Qt.ControlModifier.value, "l")
    host.key(True, Qt.Key_Backspace.value, 0)
    assert len(comp.texts) == 1
    host.key(True, Qt.Key_Z.value, Qt.ControlModifier.value, "z")
    assert len(comp.texts) == 2
    assert host.errors == []


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
    host.view.renderer.page_select([item.model._doc_key])
    host.key(True, Qt.Key_Backspace.value, 0)
    host.key(False, Qt.Key_Backspace.value, 0)
    assert len(comp.comp.texts) == before - 1
    host.key(True, Qt.Key_Z.value, Qt.ControlModifier.value, "z")     # the document's undo
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
    _run_actions(host, f"sheets:rects\x1f{key}:30:25:150:120")
    assert (plan.x_mm, plan.y_mm, plan.w_mm, plan.h_mm) == (30, 25, 150, 120)
    ws._undo()
    assert (plan.x_mm, plan.y_mm, plan.w_mm, plan.h_mm) == (20, 20, 190, 150)
    # Two items moved together: one step.
    text = comp.comp.texts[0]
    _run_actions(host, f"sheets:rects\x1f{key}:22:20:190:150;{text._doc_key}:22:190:180:9")
    assert plan.x_mm == 22 and text.x_mm == 22 and text.y_mm == 190
    ws._undo()
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
    assert host.view.renderer.page_selection() == [f._doc_key]
    ws._undo()
    assert len(comp.comp.frames) == before


@pytest.mark.parametrize("tool,where,check", [
    ("texto", "texts", lambda i: i.text == "Text" and (i.x_mm, i.y_mm) == (40.0, 200.0)),
    ("escala", "scalebars", lambda i: i.scale_n == 100.0),
    ("norte", "nortes", lambda i: (i.x_mm, i.y_mm) == (40.0, 200.0)),
    ("leyenda", "leyendas", lambda i: (i.x_mm, i.y_mm) == (40.0, 200.0)),
    ("rect", "shapes", lambda i: (i.kind, i.w_mm, i.h_mm) == ("rect", 30.0, 20.0)),
    ("terreno", "shapes", lambda i: (i.kind, i.stroke_mm, i.invert) == ("terreno", 0.5, False)),
    ("llamada", "llamadas", lambda i: i.number == "1" and i.frame_uid),
    ("perfil", "perfiles", lambda i: (i.w_mm, i.h_mm) == (80.0, 40.0)),
])
def test_the_placement_tools_make_their_item_in_one_step(host, tool, where, check):
    """A click or a drag with an item tool armed: the item there, with the
    composer's defaults (sheets::create), one step of the history, then
    back to Select."""
    ws = _sheets(host)
    comp = ws.composer
    _run_actions(host, f"sheets:tool\x1f{tool}")
    assert comp.tool_mode == tool
    before = len(getattr(comp.comp, where))
    x0, y0, x1, y1 = (40, 200, 70, 220) if tool != "llamada" else (40, 40, 60, 60)
    _run_actions(host, f"sheets:place\x1f{x0}\x1f{y0}\x1f{x1}\x1f{y1}\x1f{tool}")
    items = getattr(comp.comp, where)
    assert len(items) == before + 1 and check(items[-1])
    assert comp.tool_mode == "select"
    assert host.view.renderer.page_selection() == [items[-1]._doc_key]
    ws._undo()
    assert len(getattr(comp.comp, where)) == before


def test_a_new_text_opens_for_typing_and_the_words_are_one_edit(host):
    ws = _sheets(host)
    comp = ws.composer
    rec = host.view.renderer
    _run_actions(host, "sheets:tool\x1ftexto")
    _run_actions(host, "sheets:place\x1f40\x1f200\x1f40\x1f200\x1ftexto")
    host.view.draw_frame()
    text = comp.comp.texts[-1]
    assert rec.last("page_edit_text") == (text._doc_key,)
    _run_actions(host, f"sheets:set\x1f{text._doc_key}\x1f" + json.dumps({"text": "Planta\nbaja"}))
    assert text.text == "Planta\nbaja"
    ws._undo()
    assert text.text == "Text"


def test_the_snaps_are_the_composer_s(host):
    """What a frame offers the drawing tools (wgpu_sheet_frames.snaps: Rust
    camera and geometry) is what the composer's frame_snap_points offered:
    the same page points with the same model points behind them."""
    import numpy as np
    from views import wgpu_sheet_frames
    ws = _sheets(host)
    plan = ws.composer.comp.frames[0]
    pts, wpts, edges, ew, _circles = wgpu_sheet_frames.snaps(host.view.source_viewport, plan)
    want_pts, want_w = ws.composer.frame_snap_points(plan)
    assert len(pts) > 10
    canon = lambda p, w: sorted(tuple(np.round(np.r_[a, b], 6)) for a, b in zip(p, w))  # noqa: E731
    assert canon(pts, wpts) == canon(want_pts, want_w)
    want_e = ws.composer.frame_snap_edges(plan)
    assert len(edges) == len(want_e[0])


def test_a_dimension_measured_on_the_page_is_bound_to_the_model(host):
    """The page's ``sheets:cota``: the record in the document's units,
    measuring the two points; both on one frame's drawing, it is bound to
    that frame's model points and holds them; one step of the history; the
    tool stays armed."""
    import numpy as np
    from views import wgpu_sheet_frames
    ws = _sheets(host)
    comp = ws.composer
    plan = comp.comp.frames[0]
    pts, wpts, _e, _w, _c = wgpu_sheet_frames.snaps(host.view.source_viewport, plan)
    i = int(np.argmin(pts[:, 0] + pts[:, 1]))
    j = int(np.argmax(pts[:, 0] - pts[:, 1]))
    pt = lambda k: {"p": pts[k].tolist(), "world": wpts[k].tolist(), "frame": plan._doc_key, "segment": None}  # noqa: E731
    _run_actions(host, "sheets:tool\x1fcota")
    before = len(comp.comp.cotas)
    _run_actions(host, "sheets:cota\x1f" + json.dumps({"a": pt(i), "b": pt(j), "sep": -6.0, "axis": "h"}))
    assert len(comp.comp.cotas) == before + 1 and comp.tool_mode == "cota"
    ct = comp.comp.cotas[-1]
    assert (ct.x_mm, ct.y_mm) == tuple(pts[i]) and (ct.sep_mm, ct.axis) == (-6.0, "h")
    assert ct.anchor_uid == plan.uid and ct.a_world == wpts[i].tolist() and ct.b_world == wpts[j].tolist()
    assert ct.scale_n == plan.scale_n and ct.units == "m"
    assert getattr(ct, "hold_a", None) is not None                      # held by the model's vertex
    ws._undo()
    assert len(comp.comp.cotas) == before


def test_a_chain_s_total_stacks_one_row_beyond_its_line(host):
    ws = _sheets(host)
    comp = ws.composer
    _run_actions(host, "sheets:tool\x1fcota_cadena")
    free = lambda x, y: {"p": [x, y], "world": None, "frame": None, "segment": None}  # noqa: E731
    for a, b in (((30, 200), (60, 200)), ((60, 200), (90, 200))):
        _run_actions(host, "sheets:cota\x1f" + json.dumps({"a": free(*a), "b": free(*b), "sep": -5.0, "axis": "h"}))
    _run_actions(host, "sheets:cota_total\x1f" + json.dumps({"a": free(30, 200), "b": free(90, 200), "sep": -5.0,
                                                             "axis": "h", "p1": [30, 200], "p2": [60, 200]}))
    total = comp.comp.cotas[-1]
    assert (total.x_mm, total.dx_mm, total.axis) == (30, 60, "h")
    assert total.sep_mm == pytest.approx(-5.0 - (total.text_mm * 2.0 + 2.5))
    assert comp.tool_mode == "cota_cadena"


def test_radius_and_angle_dimensions_from_the_page(host):
    ws = _sheets(host)
    comp = ws.composer
    plan = comp.comp.frames[0]
    _run_actions(host, "sheets:tool\x1fcota_radio")
    _run_actions(host, "sheets:radial\x1f" + json.dumps({"c": [60, 60], "p": [70, 60], "kind": "diameter"}))
    r = comp.comp.cotas_rad[-1]
    assert (r.x_mm, r.y_mm, r.radius_mm, r.kind, r.angle_deg, r.scale_n) == (60, 60, 10.0, "diameter", 0.0,
                                                                              plan.scale_n)
    _run_actions(host, "sheets:tool\x1fcota_ang")
    _run_actions(host, "sheets:angular\x1f" + json.dumps({"v": [100, 230], "a": [120, 230], "b": [100, 210], "r": 8.0}))
    a = comp.comp.cotas_ang[-1]
    assert (a.ax_mm, a.ay_mm, a.bx_mm, a.by_mm, a.radius_mm) == (20, 0, 0, -20, 8.0)
    assert comp.tool_mode == "cota_ang"


def test_the_frame_s_circles_are_the_composer_s(host):
    from views import wgpu_sheet_frames
    ws = _sheets(host)
    plan = ws.composer.comp.frames[0]
    got = wgpu_sheet_frames.circles(host.view.source_viewport, plan)
    want = ws.composer.frame_circles(plan)
    assert sorted(map(tuple, got.round(6).tolist())) == sorted(tuple(round(v, 6) for v in c) for c in want)


def test_every_tool_of_the_rail_runs_on_the_page(host, monkeypatch):
    from views import wgpu_sheets
    ws = _sheets(host)
    opened = []
    monkeypatch.setattr(type(ws), "open_classic", lambda self, tool="": opened.append(tool))
    rail = {t for group in wgpu_sheets.RAIL for t in group}
    assert rail <= wgpu_sheets.NATIVE_TOOLS
    for tool in sorted(rail - {"imagen"}):
        _run_actions(host, f"sheets:tool\x1f{tool}")
    assert opened == []
    groups = host.view.renderer.last("ui_set_sheet_tools")[0]
    assert not any("classic" in t[1] for g in groups for t in g)


def test_labels_levels_polygons_and_images_from_the_page(host, monkeypatch, tmp_path):
    import numpy as np
    from PySide6.QtGui import QImage
    from views import filedialogs, wgpu_sheet_frames
    ws = _sheets(host)
    comp = ws.composer
    plan = comp.comp.frames[0]
    pts, wpts, _e, _w, _c = wgpu_sheet_frames.snaps(host.view.source_viewport, plan)
    hit = {"p": pts[0].tolist(), "world": wpts[0].tolist(), "frame": plan._doc_key, "segment": None}
    # A label: the spot (bound to the model), then where its text goes.
    _run_actions(host, "sheets:tool\x1fetiqueta")
    x0, y0 = pts[0]
    _run_actions(host, f"sheets:place\x1f{x0}\x1f{y0}\x1f250\x1f240\x1fetiqueta\x1f" + json.dumps(hit))
    label = comp.comp.etiquetas[-1]
    assert (label.x_mm, label.y_mm) == (250, 240) and label.anchor_uid == plan.uid
    assert label.a_world == wpts[0].tolist() and comp.tool_mode == "select"
    # A second label whose text lands on the first: one more leader to it.
    n = len(comp.comp.etiquetas)
    _run_actions(host, "sheets:tool\x1fetiqueta")
    _run_actions(host, "sheets:place\x1f40\x1f50\x1f251\x1f241\x1fetiqueta")
    assert len(comp.comp.etiquetas) == n and len(label.leaders) == 1
    # A level mark on the plan: the plan symbol, bound to the model.
    _run_actions(host, "sheets:tool\x1fnivel")
    _run_actions(host, f"sheets:place\x1f{x0}\x1f{y0}\x1f{x0}\x1f{y0}\x1fnivel\x1f" + json.dumps(hit))
    level = comp.comp.niveles[-1]
    assert level.symbol == "circle" and level.anchor_uid == plan.uid and (level.x_mm, level.y_mm) == (x0, y0)
    # A polygon, like the other shapes.
    _run_actions(host, "sheets:tool\x1fpoligono")
    _run_actions(host, "sheets:place\x1f30\x1f200\x1f60\x1f230\x1fpoligono")
    assert comp.comp.shapes[-1].kind == "poligono"
    # An image: the file, then the item at the margins, in its proportions.
    pic = tmp_path / "logo.png"
    img = QImage(200, 100, QImage.Format_RGB32)
    img.fill(0xff3366aa)
    img.save(str(pic))
    monkeypatch.setattr(filedialogs.file_dialogs, "getOpenFileName",
                        staticmethod(lambda *a, **k: (str(pic), "")))
    _run_actions(host, "sheets:tool\x1fimagen")
    image = comp.comp.images[-1]
    assert (image.path, image.w_mm, image.h_mm) == (str(pic), 60.0, 30.0) and comp.tool_mode == "select"
    assert np.isfinite(image.z)


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
    pymupdf = pytest.importorskip("pymupdf")
    doc = pymupdf.open(str(out))                                     # the Rust writer's
    assert doc.page_count == 1 and "Inter" in " ".join(f[3] for f in doc[0].get_fonts())
    assert "Top (plan)" in doc[0].get_text()


def test_scene_menu_reads_saved_frame_pixels_without_touching_live_lens(host, monkeypatch):
    from ingetrazo_render import render_scene_photo
    scene = host.viewport.scene
    view = scene.saved_views[0]
    lens = host.viewport.camera.lens_dict()
    valid = [str(id(scene)), str(scene.content_version), view.name]
    called = []

    def pixels(w, h, **options):
        called.append((w, h))
        assert options["scene"].edit_group is None
        assert options["camera"](None) is not host.viewport.camera
        return 2, 2, bytes((200, 2, 3, 128, 200, 2, 3, 128,
                            3, 4, 205, 200, 3, 4, 205, 200))

    monkeypatch.setattr(host.view, "render_pixels", pixels)
    assert render_scene_photo(host.window, json.dumps(["old", *valid[1:]])) is None
    assert render_scene_photo(host.window, json.dumps([valid[0], "old", valid[2]])) is None
    assert render_scene_photo(host.window, json.dumps([*valid[:2], "gone"])) is None
    assert not called
    w, h, rgba = render_scene_photo(host.window, json.dumps(valid))
    assert len(called) == 1 and (w, h) == (96, 64)
    assert list(rgba[:4]) == [200, 2, 3, 255] and list(rgba[-4:]) == [3, 4, 205, 255]
    assert host.viewport.camera.lens_dict() == lens
