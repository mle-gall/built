"""Several views of a project under Metal (views/multi_view.py with
views/wgpu_peer.py), without a GPU: a recording renderer stands in for
``ingetrazo_render.Renderer``.

The views share the document's resident scene — one pack, one upload — and
each draws on a target of its own with its own camera and style; a tool's
preview shows in one of them; Maximize/Restore and the layout's state work
as with OpenGL; closing the project hands the targets back."""
from __future__ import annotations

import os
import sys

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
import pytest
from PySide6.QtCore import QEvent
from ingetrazo_core import Vec3 as V
from PySide6.QtWidgets import QApplication

if QApplication.instance() is None:
    QApplication(sys.argv[:1])


class FakeRenderer:
    """Records each call with the target live at the time."""

    instances = []

    def __init__(self, ns_view, width, height, scale):
        self.calls = []
        self.active = 0
        self.alive = {0}
        self.next = 1
        self.sizes = {0: (width, height)}
        FakeRenderer.instances.append(self)

    # The view grid's API.
    def add_target(self, ns_view, width, height, scale):
        target, self.next = self.next, self.next + 1
        self.alive.add(target)
        self.sizes[target] = (width, height)
        self.calls.append((self.active, "add_target", (target,)))
        return target

    def bind_target(self, target):
        if target not in self.alive:
            raise ValueError("no such view target")
        self.active = target

    def remove_target(self, target):
        if target == 0 or target not in self.alive:
            raise ValueError("no such removable view target")
        self.alive.discard(target)
        if self.active == target:
            self.active = 0
        self.calls.append((self.active, "remove_target", (target,)))

    @property
    def active_target(self):
        return self.active

    # What a frame asks for an answer.
    def render(self, present=True):
        self.calls.append((self.active, "render", ()))
        return 1.0, None

    def pick(self, x, y):
        return -1

    def pick_rect(self, *a):
        return []

    gpu_memory = 1 << 20
    target_memory = 0
    pick_cache_valid = True

    def __getattr__(self, name):
        if name.startswith("__"):
            raise AttributeError(name)

        def record(*args, **kwargs):
            self.calls.append((self.active, name, args))
        return record

    def named(self, name, target=None):
        return [c for c in self.calls if c[1] == name
                and (target is None or c[0] == target)]


@pytest.fixture
def win(monkeypatch, tmp_path):
    from core import autosave
    monkeypatch.setattr(autosave, "autosave_dir", lambda: tmp_path)
    from PySide6.QtCore import QSettings
    QSettings.setPath(QSettings.IniFormat, QSettings.UserScope, str(tmp_path))
    import ingetrazo_render  # noqa: F401 — the real module, its class swapped
    from core import renderer_settings
    monkeypatch.setattr(renderer_settings, "wanted", lambda st=None: True)
    monkeypatch.setattr(renderer_settings, "arm", lambda st=None: None)
    monkeypatch.setattr(renderer_settings, "first_frame", lambda st=None: None)
    monkeypatch.setattr(ingetrazo_render, "Renderer", FakeRenderer)
    FakeRenderer.instances.clear()
    # Qt's offscreen platform cannot move a native child window to another
    # parent (it crashes): the Metal views are plain widgets here, the
    # recording renderer needing no NSView.
    from PySide6.QtCore import Qt
    from views.wgpu_view import WgpuView
    made = WgpuView.__init__

    def alien(self, *a, **k):
        made(self, *a, **k)
        for attr in (Qt.WA_NativeWindow, Qt.WA_PaintOnScreen):
            self.setAttribute(attr, False)
    monkeypatch.setattr(WgpuView, "__init__", alien)
    from views.main_window import MainWindow
    w = MainWindow()
    w.resize(1400, 800)
    w.show()
    vp = w.viewport
    vp.scene.mesh.add_face([V(0, 0, 0), V(4, 0, 0), V(4, 4, 0), V(0, 4, 0)])
    vp.scene.version += 1
    vp.notify_scene_changed()
    _settle()
    yield w
    for doc in list(w._documents):
        doc.saved_content = doc.viewport.scene.content_version
    w._saved_version = w.viewport.scene.version
    w.close()


def _settle(ms=120):
    from PySide6.QtTest import QTest
    QTest.qWait(ms)


def _renderer(win):
    from views.wgpu_peer import raw_renderer
    return raw_renderer(win._doc.stand_in.renderer)


def _four(win):
    win.set_view_layout("four")
    _settle()
    return win._doc.view_grid


def test_the_views_button_works_under_metal(win):
    from views.wgpu_peer import WgpuPeerView
    from views.wgpu_view import WgpuView
    display = win._doc.stand_in
    assert isinstance(display, WgpuView)
    grid = _four(win)
    assert grid is not None and grid.metal and grid.display is display
    assert win._doc_stack.currentWidget() is grid
    assert win._doc.slot_widget() is grid
    assert win._doc.stand_in is display, "the Metal paths still reach it"
    peers = [c.display for c in grid.cells if c.display is not display]
    assert len(peers) == 3 and all(isinstance(p, WgpuPeerView) for p in peers)
    assert sorted(c.index for c in grid.cells if not c.isHidden()) == [0, 1, 2, 3]
    # The models stay hidden: the cells show the native views.
    assert all(not c.viewport.isVisible() for c in grid.cells)
    assert all(c.display.isVisible() for c in grid.cells)


def test_extra_views_share_the_one_resident_scene(win, monkeypatch):
    import views.wgpu_view as module
    packs = []
    real = module.pack_scene
    monkeypatch.setattr(module, "pack_scene",
                        lambda *a, **k: packs.append(1) or real(*a, **k))
    renderer = _renderer(win)
    sent = len(renderer.named("set_scene"))
    grid = _four(win)
    for display in grid.displays():
        display.draw_frame()
    assert packs == [], "no second pack for the extra views"
    assert len(renderer.named("set_scene")) == sent, "nor a second upload"
    assert len(FakeRenderer.instances) == 1, "one renderer, one device"
    targets = sorted(c[2][0] for c in renderer.named("add_target"))
    assert targets == [1, 2, 3]
    # The extra views read the first view's pack, rows and snap index.
    display = grid.display
    for peer in display.peers():
        assert peer.packed is display.packed
        assert peer.uid_rows is display.uid_rows
        assert peer.renderer.raw is renderer
    assert display.renderer.target == 0


def test_each_view_draws_with_its_own_camera(win):
    renderer = _renderer(win)
    grid = _four(win)
    renderer.calls.clear()
    for display in grid.displays():
        display._last_camera = None
        display.draw_frame()
    cameras = {}
    for target, name, args in renderer.calls:
        if name == "set_camera":
            cameras[target] = args
    assert sorted(cameras) == [0, 1, 2, 3]
    assert len({tuple(view) for view, _proj in cameras.values()}) == 4
    renders = [c[0] for c in renderer.named("render")]
    assert sorted(renders) == [0, 1, 2, 3]


def test_each_view_draws_with_its_own_style(win):
    from core.style import style_by_name
    renderer = _renderer(win)
    grid = _four(win)
    top = grid.cells[0]
    top.viewport.view_style = style_by_name("X-ray").copy()
    renderer.calls.clear()
    for display in grid.displays():
        display.draw_frame()
    modes = {t: a[0].get("face_mode") for t, n, a in renderer.calls
             if n == "set_style" and isinstance(a[0], dict)}
    target = top.display.renderer.target
    assert modes.get(target) == "xray"
    assert all(mode != "xray" for t, mode in modes.items() if t != target)


def test_a_tools_preview_shows_in_the_view_under_the_pointer(win):
    renderer = _renderer(win)
    grid = _four(win)
    win._activate_tool("line")
    grid.set_tool_view(grid.cells[2].viewport)
    for display in grid.displays():
        display.draw_frame()
    here = grid.cells[2].display
    assert not here._preview_given_away
    # Each other view emptied its own preview when it gave it away.
    last = {}
    for target, name, args in renderer.named("set_preview_mesh"):
        last[target] = all(len(x) == 0 for x in args)
    for cell in grid.cells:
        if cell.index != 2:
            assert cell.display._preview_given_away
            assert last[cell.display.renderer.target], cell.index
    assert [c.index for c in grid.cells if c.viewport.tool_preview] == [2]
    win._activate_tool("select")


def test_the_pointer_entering_a_metal_view_brings_the_tool_and_preview(win):
    grid = _four(win)
    win._activate_tool("line")
    cell = grid.cells[3]
    QApplication.sendEvent(cell.display, QEvent(QEvent.Enter))
    assert cell.viewport.active_tool is win.viewport.active_tool
    assert [c.index for c in grid.cells if c.viewport.tool_preview] == [3]
    tool = win.viewport.active_tool
    assert cell.display.tools.tool is tool
    if hasattr(tool, "_viewport"):
        assert tool._viewport is cell.display.tools
    win._activate_tool("select")


def test_maximize_and_restore_under_metal(win):
    renderer = _renderer(win)
    grid = _four(win)
    grid.toggle_maximized(grid.cells[3])
    _settle()
    assert [c.index for c in grid.cells if not c.isHidden()] == [3]
    assert not grid.display.isVisible()
    # The first view hidden, the document still changes and draws.
    sent = len(renderer.named("set_scene")) + len(renderer.named("update_instances"))
    vp = win.viewport
    vp.scene.mesh.add_face([V(6, 0, 0), V(8, 0, 0), V(8, 2, 0), V(6, 2, 0)])
    vp.scene.version += 1
    vp.notify_scene_changed()
    _settle(200)
    renderer.calls.clear()
    grid.cells[3].display.draw_frame()
    assert [c[0] for c in renderer.named("render")] == [grid.cells[3].display.renderer.target]
    assert grid.display._scene_dirty is False
    grid.toggle_maximized(grid.cells[3])
    _settle()
    assert sorted(c.index for c in grid.cells if not c.isHidden()) == [0, 1, 2, 3]
    assert sent >= 1


def test_the_layout_is_saved_and_comes_back(win, tmp_path):
    grid = _four(win)
    grid.cells[0].pick_view("front")
    state = win._views_dict(win._doc)
    assert state["layout"] == "four"
    project = grid.layout_of_project()
    assert project["layout"] == "four" and project["views"][0][0] == "front"
    win.set_view_layout("two")
    _settle()
    assert sorted(c.index for c in grid.cells if not c.isHidden()) == [0, 1]
    win.set_view_layout("single")
    _settle()
    assert [c.index for c in grid.cells if not c.isHidden()] == [1]
    assert grid.display.isVisible()


def test_closing_the_project_hands_the_targets_back(win):
    renderer = _renderer(win)
    grid = _four(win)
    doc = win._doc
    win._new_document()
    _settle()
    display = doc.stand_in
    doc.viewport.scene.version += 1
    doc.viewport.notify_scene_changed()
    assert display._refresh_timer.isActive()
    win._retire_document(doc)
    assert not display._refresh_timer.isActive()
    assert not display._repaint_timer.isActive()
    _settle()
    removed = sorted(c[2][0] for c in renderer.named("remove_target"))
    assert removed == [1, 2, 3]
    assert renderer.alive == {0}
