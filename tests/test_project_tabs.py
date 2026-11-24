# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Several projects open in one window, each in its own tab (#1066).

Each tab is a project with its own viewport — scene, undo history,
camera, saved state, auto-save slot — and ``window.viewport`` answers the
one in front. These tests drive the real window: open in a tab, reuse an
untouched one, jump to a file already open, switch, close, quit, auto-save
and recovery, the extension hooks, copy and paste between projects, and
that a closed project lets its memory go."""
from __future__ import annotations

import gc
import time
import weakref


import pytest
from ingetrazo_core import Vec3 as V

from formats import igz as igz_format
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import wgpu_tabs


def _face(scene, z=0.0, size=2.0):
    scene.mesh.add_face([V(0, 0, z), V(size, 0, z), V(size, size, z),
                         V(0, size, z)])
    scene.version += 1


def _igz(tmp_path, name, z=0.0):
    from core.scene import Scene
    s = Scene()
    _face(s, z)
    path = tmp_path / name
    igz_format.save_scene(s, path)
    return path


def _edit(win, z=5.0):
    """One real, undoable edit in the project in front."""
    from core.history import AddEdgeCommand
    win.viewport.history.execute(AddEdgeCommand(V(0, 0, z), V(1, 0, z)))
    win.viewport.notify_scene_changed()


def _clean_all(win):
    for d in win.documents():
        d.saved_content = d.viewport.scene.content_version


@pytest.fixture
def win(native_host, tmp_path, monkeypatch):
    from core import autosave
    slots = tmp_path / "autosave"
    slots.mkdir()
    monkeypatch.setattr(autosave, "autosave_dir", lambda: slots)
    return native_host.window


# ---- opening -------------------------------------------------------------
def test_one_tab_at_start_and_the_bar_is_shown(win):
    assert len(win.documents()) == 1
    assert win._project_tabs.count() == 1
    assert wgpu_tabs.tabs(win)[0][2]
    assert win.viewport is win.active_document().viewport
    assert win._project_tabs.tabs.tabText(0) == "Untitled"


def test_an_untouched_tab_is_reused_by_open(win, tmp_path):
    first = win.viewport
    assert win.open_path(_igz(tmp_path, "a.igz"))
    assert len(win.documents()) == 1
    assert win.viewport is first
    assert win._project_tabs.tabs.tabText(0) == "a.igz"


def test_open_comes_in_a_new_tab_when_the_front_one_has_work(win, tmp_path):
    _edit(win)
    mine = win.active_document()
    path = _igz(tmp_path, "b.igz", z=3.0)
    assert win.open_path(path)
    assert len(win.documents()) == 2
    assert win.active_document() is not mine
    assert win._current_path == path
    # The unsaved work is still there, in its tab, still unsaved.
    assert mine.path is None and win._doc_is_dirty(mine)
    assert len(mine.viewport.history.undo_stack) == 1
    # The new tab sits right of the one it was opened from.
    assert win.documents() == [mine, win.active_document()]


def test_a_file_already_open_brings_its_tab_back(win, tmp_path):
    a = _igz(tmp_path, "a.igz")
    b = _igz(tmp_path, "b.igz")
    win.open_path(a)
    doc_a = win.active_document()
    win.open_path(b)
    assert len(win.documents()) == 2
    # Edited meanwhile: jumping back must not reread it from disk.
    win.activate_document(doc_a)
    _edit(win)
    win.activate_document(win.documents()[1])
    assert win.open_path(a)
    assert win.active_document() is doc_a
    assert len(win.documents()) == 2
    assert win._doc_is_dirty(doc_a)


def test_a_failed_open_leaves_no_empty_tab_behind(win, tmp_path, monkeypatch):
    _edit(win)
    mine = win.active_document()
    bad = tmp_path / "bad.igz"
    bad.write_text("not a document")
    monkeypatch.setattr("views.chrome_dialogs.critical", lambda *a, **k: None)
    assert not win.open_path(bad)
    assert win.documents() == [mine]
    assert win.active_document() is mine


def test_file_new_opens_a_tab_and_reuses_an_untouched_one(win):
    win._on_new()                       # untouched: started over in place
    assert len(win.documents()) == 1
    _edit(win)
    win._on_new()
    assert len(win.documents()) == 2
    assert win._project_tabs.tabs.tabText(1) == "Untitled"
    assert not win._is_dirty()
    # A new project starts with its own scale figure, clean.
    assert win.viewport.scene.groups


def test_the_plus_button_makes_a_project(win):
    _edit(win)
    wgpu_tabs.action(win, win.view, "new")
    assert len(win.documents()) == 2


# ---- switching -----------------------------------------------------------
def test_each_tab_keeps_its_own_undo_and_camera(win, tmp_path):
    _edit(win, z=1.0)
    a = win.active_document()
    a.viewport.camera.distance = 77.0
    win._on_new()
    b = win.active_document()
    _edit(win, z=2.0)
    _edit(win, z=3.0)
    assert len(b.viewport.history.undo_stack) == 2
    win._on_undo()
    assert len(b.viewport.history.undo_stack) == 1
    assert len(a.viewport.history.undo_stack) == 1   # untouched by B's undo
    win.activate_document(a)
    assert win.viewport.camera.distance == 77.0
    win._on_undo()
    assert not a.viewport.history.undo_stack
    assert len(b.viewport.history.undo_stack) == 1


def test_switching_drops_the_tool_and_leaves_the_group(win):
    from core.group import Group
    from core.mesh import Mesh
    m = Mesh()
    m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    g = Group(m, "box")
    win.viewport.scene.groups.append(g)
    win.viewport.scene.version += 1
    win._saved_version = -1                       # some work: not reusable
    a = win.active_document()
    win.viewport.begin_group_edit(g)
    assert win.viewport.scene.edit_group is g
    win._activate_tool("line")
    win._on_new()
    assert a.viewport.active_tool is None
    assert a.viewport.scene.edit_group is None
    assert win.viewport.active_tool is win._tools["select"]
    win.activate_document(a)
    assert win.viewport.active_tool is win._tools["select"]


def test_the_title_and_the_tab_mark_follow_the_front(win, tmp_path):
    path = _igz(tmp_path, "casa.igz")
    win.open_path(path)
    assert win._host.idle()[3] == "casa.igz — Built"
    _edit(win)
    assert win._host.idle()[3] == "casa.igz * — Built"
    assert win._project_tabs.tabs.tabText(0) == "casa.igz •"
    win._on_new()
    assert win._host.idle()[3] == "Untitled — Built"
    # The background tab still says it has unsaved changes.
    assert win._project_tabs.tabs.tabText(0) == "casa.igz •"


def test_ctrl_tab_walks_the_projects(win):
    single = win.active_document()
    win.run_command("next_doc")
    assert win.active_document() is single
    second = win.new_document()
    win.run_command("next_doc")
    assert win.active_document() is single
    win.run_command("prev_doc")
    assert win.active_document() is second


def test_panels_forget_what_they_drew_for_the_other_project(win):
    from tests.test_tree_links import _View
    from views import wgpu_ui
    from tests.test_in_model_nested import _component, _quad
    view = _View()
    for name in ("First project component", "Second project component"):
        win.new_document()
        group = _component(_quad(), name)
        win.viewport.scene.groups.append(group)
        win.viewport.scene.version += 1
        assert [r[0] for r in wgpu_ui._components_in_model(view, win)] == [name]
        wgpu_ui._tree(view, win)
        assert view._ui_tree_model[0][0] == id(win.viewport.scene)


def test_materials_let_go_of_the_meshes_of_the_project_left(win):
    from core.group import Group
    from core.mesh import Mesh
    from views.wgpu_library import material_uses
    mesh = Mesh()
    face = mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])
    face.attrs = {"mat": "Brick"}
    win.viewport.scene.groups.append(Group(mesh, "Wall"))
    win.viewport.scene.version += 1
    assert material_uses(win.viewport.scene)["Brick"]
    old_mesh = weakref.ref(mesh)
    doc = win.active_document()
    win.new_document()
    assert not material_uses(win.viewport.scene)["Brick"]
    del mesh, face
    assert win.close_document(doc, ask=False)
    win._host.idle()
    assert old_mesh() is None


def test_view_preferences_reach_the_tabs_behind(win, tmp_path):
    from views.settings_sheet import Env, SettingsSheet

    _edit(win)
    a = win.active_document()
    win._on_new()
    env = Env(win)
    want = not a.viewport._invert_wheel
    row = next(r for _, _, rows in SettingsSheet().groups(env, "navigation") for r in rows
               if r.id == "invert_wheel")
    row.put(env, want)
    assert a.viewport._invert_wheel == want
    assert win.viewport._invert_wheel == want


# ---- closing -------------------------------------------------------------
def test_a_closed_project_lets_its_memory_go(win, tmp_path):
    _edit(win)
    assert win.open_path(_igz(tmp_path, "big.igz"))
    scene = weakref.ref(win.viewport.scene)
    vp = weakref.ref(win.viewport)
    assert win.close_document()
    win._host.idle()
    assert vp() is None and scene() is None


def test_closing_the_first_tab_lets_it_go_too(win, tmp_path):
    assert win.open_path(_igz(tmp_path, "first.igz"))
    vp = weakref.ref(win.viewport)
    scene = weakref.ref(win.viewport.scene)
    assert win.close_document(ask=False)
    win._host.idle()
    assert vp() is None and scene() is None


def test_closing_tabs_hands_their_memory_back_once(win, tmp_path, monkeypatch):
    from core import malloc_arenas
    trims = []
    monkeypatch.setattr(malloc_arenas, "trim", lambda: trims.append(1))
    scenes = []
    for name in ("a.igz", "b.igz"):
        assert win.open_path(_igz(tmp_path, name))
        scenes.append(weakref.ref(win.viewport.scene))
        monkeypatch.setattr(type(win.view), "render_pixels",
                            lambda *a, **k: pytest.fail("redrew a closing tab"))
    was_enabled = gc.isenabled()
    gc.disable()
    try:
        assert win.close_documents(win.documents())
        assert not trims
        win._host.idle()
        assert all(ref() is None for ref in scenes)
        assert trims == [1]
    finally:
        if was_enabled:
            gc.enable()


# ---- auto-save and recovery ---------------------------------------------
def test_each_untitled_project_autosaves_to_its_own_slot(win):
    from core import autosave
    _edit(win, z=1.0)
    a = win.active_document()
    win._on_new()
    _edit(win, z=2.0)
    b = win.active_document()
    assert a.untitled_no != b.untitled_no
    try:
        win._on_autosave_tick()
        import time
        deadline = time.monotonic() + 10
        while win._background_autosave.busy and time.monotonic() < deadline:
            win._host.idle()
            time.sleep(0.005)
        assert not win._background_autosave.busy
        slot_a = autosave.pending(None, untitled=a.untitled_no)
        slot_b = autosave.pending(None, untitled=b.untitled_no)
        assert slot_a and slot_b and slot_a != slot_b
        # Each slot holds its own project (the tab behind too).
        from core.scene import Scene
        for slot, z in ((slot_a, 1.0), (slot_b, 2.0)):
            s = Scene()
            igz_format.load_into(s, slot)
            assert any(abs(e.v1.position.z() - z) < 1e-6
                       for e in s.mesh.edges)
        # Closing a tab retires only its slot.
        _clean_all(win)
        win.close_document(b)
        assert autosave.pending(None, untitled=b.untitled_no) is None
        assert autosave.pending(None, untitled=a.untitled_no) is not None
    finally:
        for n in (a.untitled_no, b.untitled_no):
            autosave.clear(None, untitled=n)


def test_every_untitled_slot_is_offered_back(win, monkeypatch, tmp_path):
    from core import autosave
    from core.scene import Scene
    for n, z in ((1, 1.0), (2, 2.0)):
        s = Scene()
        _face(s, z)
        igz_format.save_scene(s, autosave.slot_for(None, untitled=n))
    try:
        monkeypatch.setattr("views.chrome_dialogs.ask",
                            lambda *a, **k: "yes")
        win._offer_untitled_recovery()
        docs = win.documents()
        assert len(docs) == 2
        zs = sorted(next(iter(d.viewport.scene.mesh.faces)).vertices[0].z()
                    for d in docs)
        assert zs == [1.0, 2.0]
        assert sorted(d.untitled_no for d in docs) == [1, 2]
        assert all(win._doc_is_dirty(d) for d in docs)
    finally:
        for n in (1, 2):
            autosave.clear(None, untitled=n)


def test_the_classic_slot_names_stay(tmp_path):
    from core import autosave
    assert autosave.slot_for(None).name == "untitled.built"
    assert autosave.slot_for(None, untitled=1).name == "untitled.built"
    assert autosave.slot_for(None, untitled=3).name == "untitled-3.built"


# ---- extensions ----------------------------------------------------------
def test_extension_hooks_follow_the_tabs(win, tmp_path, monkeypatch):
    from views.extension_api import ExtensionApp
    app = ExtensionApp(win, "probe")
    changed, closed = [], []
    app.on_document_changed(lambda: changed.append(app.scene))
    app.on_document_closed(lambda scene: closed.append(scene))
    drawn = []
    app.add_overlay(lambda vp, p: drawn.append(vp))
    _edit(win)
    a_scene = win.viewport.scene
    win._on_new()
    b_scene = win.viewport.scene
    assert changed and changed[-1] is b_scene    # the switch is a change
    assert app.scene is b_scene and app.viewport is win.viewport
    # Registered once, the overlay is in the new tab's viewport too.
    assert win.viewport._ext_overlays is win.documents()[0].viewport._ext_overlays
    n = len(changed)
    _edit(win)                                    # an edit in the new tab
    assert len(changed) > n
    monkeypatch.setattr(win, "_confirm_discard", lambda prompt: True)
    win.close_document(win.active_document())
    assert closed == [b_scene]
    assert app.scene is a_scene


def test_the_ai_bridge_acts_on_the_tab_in_front(win):
    from plugins.ai_bridge import BridgeSection
    section = BridgeSection(win.viewport)
    first = win.viewport
    _edit(win)
    win._on_new()
    assert section._viewport is win.viewport
    assert section._viewport is not first


def test_a_workspace_holds_the_tabs(win):
    from core.history import History
    from core.scene import Scene

    class _Ws:
        def __init__(self):
            self.scene = Scene()
            self.history = History(self.scene)

        def title(self):
            return "job"

        def is_dirty(self):
            return False

        def save(self):
            pass

        def save_as(self):
            pass

        def confirm_leave(self):
            return True

    _edit(win)
    a = win.active_document()
    win._on_new()
    win.enter_workspace(_Ws())
    win.activate_document(a)                      # refused while it is shown
    assert win.active_document() is not a
    assert win.leave_workspace()
    win.activate_document(a)
    assert win.active_document() is a


# ---- copy and paste between projects --------------------------------------
def test_copy_in_one_project_paste_in_another(win):
    from core.group import Group
    from core.mesh import Mesh
    vp = win.viewport
    m = Mesh()
    m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    g = Group(m, "Banca")
    vp.scene.groups.append(g)
    vp.scene.version += 1
    win._saved_version = -1
    vp.scene.selection.clear()
    vp.scene.selection.add(g)
    vp.copy_selection()
    a_clip = vp.clipboard
    assert a_clip and a_clip["groups"]
    win._on_new()
    b = win.viewport
    assert b.clipboard is None
    before = len(b.scene.groups)
    win._sync_foreign_clipboard()
    clip = b.clipboard
    assert clip and clip is not a_clip
    # Through the document encoding: the pasting project's own copy, no
    # object shared with the other document.
    assert clip["groups"][0] is not a_clip["groups"][0]
    assert clip["groups"][0].mesh is not g.mesh
    from tools.paste import PasteTool
    assert PasteTool.in_place(b)
    assert len(b.scene.groups) == before + 1
    assert any(gr.name == "Banca" for gr in b.scene.groups)
    assert win.documents()[0].viewport.scene.groups.count(g) == 1
    # A second paste reuses the adopted copy, no new decoding.
    win._sync_foreign_clipboard()
    assert b.clipboard is clip


def test_open_progress_is_one_monotonic_lifecycle(win, tmp_path, monkeypatch):
    path = _igz(tmp_path, "progress.igz")
    progress = []
    bars = []
    monkeypatch.setattr("views.chrome_dialogs.live", lambda *_: win.view)
    original = win._import_progress

    def dialog(title, **kwargs):
        bar, update = original(title, **kwargs)
        bars.append(bar)
        def record(fraction, message):
            update(fraction, message)
            progress.append(bar.value())
        return bar, record

    monkeypatch.setattr(win, "_import_progress", dialog)
    monkeypatch.setattr(type(win.view), "render_pixels", lambda *a, **k: pytest.fail("eager drawing"))
    assert win.open_path(path)
    assert progress and progress == sorted(progress) and progress[-1] == 100
    assert all(value < 100 for value in progress[:-1])
    assert not bars[0].isVisible() and not win._open_in_progress
