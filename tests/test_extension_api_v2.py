# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The extension API, version 2 (views.extension_api) — what a bigger
extension needs beyond Levels: a CAM job, say, which has its own panels,
draws toolpaths over the view, opens its own files and shows its own
document in place of the model.

- **Panels** retain a stable identity in the native plugin host.
- **Overlays** get the vectorised projection; one that raises is logged
  once and removed, and the painter state never leaks.
- **File openers**: a document of the extension's type goes to it from
  Open Recent, the command line or a double-click.
- **Workspaces**: the model is parked, not closed, and comes back with its
  geometry, undo steps, file, camera and saved state; meanwhile the File
  actions, the title, the prompts, autosave and the tool set belong to the
  workspace, and the viewport's id()-keyed caches are reset at each swap.
"""
from __future__ import annotations

from pathlib import Path

import numpy as np
import pytest
from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
from built_render import AnnotationCanvas
from built_core import Vec3 as V

from core.history import History
from core.scene import Scene

def _app_for(win, key="myext"):
    from views.extension_api import ExtensionApp
    return ExtensionApp(win, key)


def test_the_api_version_stays_2():
    # Projects in tabs (#1066) only add to version 2, and published
    # extensions (loop_cut, vertex_editor) refuse anything but 1 or 2: a 3
    # would turn them away although nothing they use changed.
    from views.extension_api import API_VERSION, ExtensionApp
    assert API_VERSION == 2 and ExtensionApp.api_version == 2
    assert hasattr(ExtensionApp, "on_document_closed")


def test_extensions_can_import_the_complete_native_v2_api():
    import __future__
    import importlib
    import logging

    import built_render
    from formats import built

    api = importlib.import_module("views.extension_api")
    # Published plugins could import any of these names, including with import *.
    exports = {}
    exec("from views.extension_api import *", {}, exports)
    assert {
        "API_VERSION", "CORE_FILE_SUFFIXES", "ExtensionApp", "Job", "annotations",
        "built", "chrome_dialogs", "chrome_forms", "log", "logging",
    } <= exports.keys()
    for name in ("chrome_forms", "chrome_dialogs"):
        assert exports[name] is importlib.import_module(f"views.{name}")
        assert exports[name] is getattr(built_render, name)
    assert exports["Job"] is built_render.chrome_dialogs.Job
    assert exports["annotations"] is __future__.annotations
    assert exports["built"] is built
    assert exports["logging"] is logging
    assert exports["log"] is logging.getLogger("built.plugins")
    assert api.ExtensionApp.__module__ == "views.extension_api"
    app = api.ExtensionApp(object(), "example")
    assert app.key == "example"
    assert {
        "add_agent_tool", "add_command_listener", "add_context_menu", "add_file_opener",
        "add_menu", "add_menu_action", "add_outliner_kind", "add_overlay", "add_panel",
        "add_pickable", "add_sheet_annotations", "add_snap_provider", "api_version",
        "document_data", "enter_workspace", "import_igz", "leave_workspace",
        "on_document_changed", "on_document_closed", "pick_point", "release_pick",
        "remove_agent_tools", "restore_toolbar_layout", "scene", "set_document_data",
        "show_panel", "viewport", "viewport_widget", "window", "workspace", "world_to_pixels",
    } <= set(dir(api.ExtensionApp))


@pytest.mark.parametrize("first", ["views.extension_api", "views.chrome_forms", "views.chrome_dialogs"])
def test_plugin_api_imports_in_a_fresh_interpreter(first):
    import os
    import subprocess
    import sys

    script = f"""import {first}
from views import extension_api, chrome_forms, chrome_dialogs
from views.extension_api import ExtensionApp, Job
import built_render

assert ExtensionApp(object(), "example").api_version == 2
assert extension_api.chrome_forms is chrome_forms is built_render.chrome_forms
assert extension_api.chrome_dialogs is chrome_dialogs is built_render.chrome_dialogs
assert Job is chrome_dialogs.Job
"""
    env = os.environ.copy()
    env.pop("PYTHONPATH", None)
    subprocess.run([sys.executable, "-c", script], cwd=Path(__file__).resolve().parents[1],
                   env=env, check=True, timeout=20)


# ---- panels -----------------------------------------------------------------------

def test_reopening_plugin_panels_preserves_each_instance(native_host, monkeypatch):
    import json
    from types import SimpleNamespace
    from unittest.mock import Mock
    from core import plugin_panels

    win, view = native_host.window, native_host.view
    renderer = Mock()
    monkeypatch.setattr(view, "renderer", renderer)
    monkeypatch.setattr(view, "_ui_attached", True, raising=False)

    def factory(app, invalidate):
        return SimpleNamespace(id="main", title=app.key, slot="dock", sections=lambda: [])

    win._panel_factories = {}
    app = _app_for(win)
    first_key = app.add_panel("First", factory, panel="first")
    assert app.show_panel(first_key)
    second_key = app.add_panel("Second", factory, panel="second")
    assert app.show_panel(second_key)
    assert app.show_panel(first_key)
    sent = [json.loads(call.args[0]) for call in renderer.ui_plugin_panel.call_args_list]
    assert [(s["plugin"], s["panel"]) for s in sent] == [("first", "main"), ("second", "main")]
    first, second = (s["instance"] for s in sent)
    assert first != second
    assert [call.args[0] for call in renderer.ui_plugin_open.call_args_list] == [first, second, first]


# ---- overlays -----------------------------------------------------------------------

def _frame(vp):
    """One overlay pass on an image (offscreen has no GL frame to paint)."""
    painter = AnnotationCanvas(200, 200, 200)
    vp._draw_extension_overlays(painter)
    return painter


def test_an_overlay_draws_and_never_leaks_painter_state(native_window):
    win = native_window
    vp = win.viewport
    seen = []

    def paint(viewport, painter):
        seen.append(viewport)
        font = painter.font()
        font.setPointSize(64)
        painter.setFont(font)

    def after(viewport, painter):
        seen.append(painter.fontMetrics().height())

    app = _app_for(win)
    app.add_overlay(paint)
    app.add_overlay(after)
    _frame(vp)
    assert seen == [vp, AnnotationCanvas(200, 200, 200).fontMetrics().height()]


def test_a_failing_overlay_is_dropped_once_and_the_others_still_run(native_window, caplog):
    win = native_window
    vp = win.viewport
    ran = []

    def broken(_vp, _p):
        raise RuntimeError("boom")

    def fine(_vp, _p):
        ran.append(1)

    app = _app_for(win)
    app.add_overlay(broken)
    app.add_overlay(fine)
    _frame(vp)
    _frame(vp)
    assert broken not in vp._ext_overlays and fine in vp._ext_overlays
    assert len(ran) == 2
    assert sum("failed; removed" in r.message for r in caplog.records) == 1


def test_the_vectorised_projection_matches_the_per_point_one(native_window):
    win = native_window
    vp = win.viewport
    pts = np.array([[0.0, 0.0, 0.0], [1.0, 2.0, 0.5], [-3.0, 1.0, 2.0]])
    px, py, front = _app_for(win).world_to_pixels(pts)
    for i, p in enumerate(pts):
        one = vp.world_to_pixel(V(*p))
        if one is None:
            assert not front[i]
        else:
            assert front[i]
            assert px[i] == pytest.approx(one[0], abs=1e-3)   # float32 vs float64
            assert py[i] == pytest.approx(one[1], abs=1e-3)


# ---- file openers ----------------------------------------------------------------------

def test_an_extension_opens_its_own_file_type(native_window):
    win = native_window
    opened = []
    _app_for(win).add_file_opener("XYZ", lambda p: opened.append(p) or True)
    assert ".xyz" in win.file_openers
    assert win.open_path(Path("/tmp/job.XYZ"))
    assert opened == [Path("/tmp/job.XYZ")]


@pytest.mark.parametrize("suffix", [".igz", ".builtproject", ".igzproj", ".gltf", ".dae", ".skp", ".dxf", ".dwg",
                                    ".obj", ".stl", ".glb"])
def test_a_core_suffix_cannot_be_claimed(native_window, caplog, suffix):
    win = native_window
    _app_for(win).add_file_opener(suffix, lambda p: True)
    assert suffix not in win.file_openers
    assert any("core format" in r.message for r in caplog.records)


def test_a_suffix_already_claimed_is_refused_not_overwritten(native_window, caplog):
    win = native_window
    app = _app_for(win, key="first")
    first_opened = []
    app.add_file_opener(".xyz", lambda p: first_opened.append(p) or True)
    second_opened = []
    _app_for(win, key="second").add_file_opener(
        ".xyz", lambda p: second_opened.append(p) or True)
    assert win.open_path(Path("/tmp/job.xyz"))
    assert first_opened == [Path("/tmp/job.xyz")]
    assert second_opened == []
    assert any("already taken" in r.message for r in caplog.records)


def test_the_launcher_hands_an_extension_file_to_the_window(native_host, tmp_path, monkeypatch):
    opened = []
    _app_for(native_host.window).add_file_opener(".xyz", lambda path: opened.append(path) or True)
    monkeypatch.setattr("built_render.chrome_dialogs.warning", lambda *args, **kwargs: None)
    assert native_host.open_path(tmp_path / "a.xyz")
    assert not native_host.open_path(tmp_path / "b.unknown")
    assert opened == [tmp_path / "a.xyz"]


# ---- workspaces -----------------------------------------------------------------------

class FakeWorkspace:
    def __init__(self, allowed=None, leave=True):
        self.scene = Scene()
        self.history = History(self.scene)
        self.allowed_tools = allowed
        self.saved = 0
        self.dirty = False
        self.leave_answer = leave
        self.gone = False

    def title(self):
        return "job.xyz"

    def is_dirty(self):
        return self.dirty

    def save(self):
        self.saved += 1
        self.dirty = False

    def save_as(self):
        self.save()

    def confirm_leave(self):
        return self.leave_answer

    def left(self):
        self.gone = True


def test_the_model_is_parked_and_comes_back(native_window):
    win = native_window
    app = _app_for(win)
    model = win.viewport.scene
    model.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    history = win.viewport.history
    win._saved_version = model.version                   # a saved model
    ws = FakeWorkspace()
    assert app.enter_workspace(ws)
    assert app.workspace() is ws
    assert win.viewport.scene is ws.scene and win.viewport.history is ws.history
    assert ws.scene.version > model.version              # no render cache is shared
    assert win._host._title == "Built — job.xyz"
    assert not app.enter_workspace(FakeWorkspace())      # one at a time
    assert app.leave_workspace()
    assert win.viewport.scene is model and win.viewport.history is history
    assert len(model.mesh.faces) == 1
    assert not win._is_dirty()                           # still saved
    assert ws.gone and app.workspace() is None


def test_workspace_cannot_take_over_an_open_or_write_over_the_parked_model(native_window, tmp_path):
    from built_render import shell_windows as windows
    from built_render import shell_files as files
    win = native_window
    doc = win.active_document()
    ws = FakeWorkspace()
    win._open_in_progress = True
    assert not win.enter_workspace(ws)
    win._open_in_progress = False
    assert win.enter_workspace(ws)
    assert not windows.can_move(win)
    target = tmp_path / "parked.built"
    target.write_bytes(b"parked model")
    assert not files.do_save(win, target)
    assert target.read_bytes() == b"parked model"
    assert win.leave_workspace() and win.active_document() is doc


def test_each_swap_resets_the_document_caches(native_window, monkeypatch):
    win = native_window
    calls = []
    orig = win.viewport.reset_document_caches
    monkeypatch.setattr(win.viewport, "reset_document_caches",
                        lambda: (calls.append(1), orig())[1])
    app = _app_for(win)
    app.enter_workspace(FakeWorkspace())
    app.leave_workspace()
    assert len(calls) == 2


def test_file_actions_and_the_title_go_to_the_workspace(native_window):
    win = native_window
    ws = FakeWorkspace()
    _app_for(win).enter_workspace(ws)
    ws.dirty = True
    win._update_title()
    assert win._host._title.endswith("job.xyz *")
    assert win._is_dirty()
    win._on_save()
    assert ws.saved == 1 and not win._is_dirty()
    win.leave_workspace()


def test_the_tool_filter_keeps_other_tools_out(native_window):
    win = native_window
    ws = FakeWorkspace(allowed={"select", "line", "rectangle"})
    _app_for(win).enter_workspace(ws)
    assert not win._tool_actions["pushpull"].isEnabled()
    assert win._tool_actions["line"].isEnabled()
    win._activate_tool("pushpull")
    assert win.viewport.active_tool is not win._tools["pushpull"]
    win.leave_workspace()
    assert win._tool_actions["pushpull"].isEnabled()


def test_a_workspace_that_will_not_go_stops_quitting(native_window):
    win = native_window
    ws = FakeWorkspace(leave=False)
    _app_for(win).enter_workspace(ws)
    from built_render.shell_files import request_close
    assert not request_close(win)
    assert win.workspace() is ws
    ws.leave_answer = True
    assert win.leave_workspace()


def test_opening_an_igz_leaves_the_workspace_first(native_window, tmp_path):
    win = native_window
    from formats import igz
    doc = tmp_path / "model.igz"
    igz.save_scene(Scene(), doc)
    ws = FakeWorkspace()
    _app_for(win).enter_workspace(ws)
    assert win.open_path(doc)
    assert win.workspace() is None and ws.gone


def test_autosave_pauses_while_the_model_is_parked(native_window, monkeypatch):
    win = native_window
    from built_core import autosave
    written = []
    monkeypatch.setattr(autosave, "write", lambda *a, **k: written.append(a))
    ws = FakeWorkspace()
    _app_for(win).enter_workspace(ws)
    ws.dirty = True
    win._on_autosave_tick()
    assert written == []
    win.leave_workspace()
