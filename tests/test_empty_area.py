# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Start, new, open, drop and edit after closing the last native document."""
from __future__ import annotations

import pytest

from tests.shell_isolation import native_host, restore_shell_patches  # noqa: F401
from ingetrazo_render import wgpu_start
from views import wgpu_tabs


@pytest.fixture
def win(native_host):
    return native_host.window


def _doc_file(tmp_path, name="abierto.igz"):
    from core.scene import Scene
    from formats import igz
    path = tmp_path / name
    igz.save_scene(Scene(), path)
    return path


def test_closing_the_last_project_shows_start(win):
    assert win.close_document()
    assert wgpu_start.shown(win)
    assert not wgpu_start.can_return(win)


def test_plus_brings_a_project_and_its_scene_commands_back(win):
    from core import units
    units.set_ask_at_start(False)
    win.close_document()
    wgpu_tabs.action(win, None, "new")
    assert not wgpu_start.shown(win)
    assert len(win.documents()) == 1
    assert not win.viewport.scene.saved_views
    win._host.menu("scene_add")
    assert len(win.viewport.scene.saved_views) == 1


def test_start_claims_modelling_shortcuts_until_a_document_opens(win):
    from PySide6.QtCore import QEvent, Qt
    from PySide6.QtGui import QKeyEvent
    from ingetrazo_render import chrome_sheet

    win.close_document()
    key = QKeyEvent(QEvent.KeyPress, Qt.Key_L, Qt.NoModifier, "l")
    assert chrome_sheet.claims_key(win._host.view, key)
    wgpu_start.new_model(win, ask_units=False)
    assert not chrome_sheet.claims_key(win._host.view, key)


def test_file_new_leaves_start(win):
    win.close_document()
    win._host.menu("new")
    assert not wgpu_start.shown(win)
    assert len(win.documents()) == 1


def test_opening_a_file_leaves_start(win, tmp_path):
    path = _doc_file(tmp_path)
    win.close_document()
    assert win.open_path(path)
    assert not wgpu_start.shown(win)
    assert len(win.documents()) == 1 and win._doc.path == path


def test_an_unreadable_document_leaves_start_open(win, tmp_path):
    bad = tmp_path / "broken.igz"
    bad.write_text("not a document")
    win.close_document()
    assert not win.open_path(bad)
    assert wgpu_start.shown(win)


def test_a_file_dropped_on_start_opens(win, tmp_path):
    path = _doc_file(tmp_path, "dropped.igz")
    win.close_document()
    # winit's DroppedFile and Finder's Open With both call Host.open_path.
    assert win._host.open_path(path)
    assert not wgpu_start.shown(win) and win._doc.path == path


def test_an_unsupported_drop_leaves_start_open(win, tmp_path):
    other = tmp_path / "notes.txt"
    other.write_text("x")
    win.close_document()
    assert not win._host.open_path(other)
    assert wgpu_start.shown(win)


def test_closing_all_from_the_tab_menu_shows_start(win):
    win.new_document()
    wgpu_tabs.action(win, None, "menu:close_all:1")
    assert wgpu_start.shown(win)
    assert not wgpu_start.can_return(win)


def test_plus_from_start_inserts_the_scale_figure(win):
    win.close_document()
    wgpu_tabs.action(win, None, "new")
    assert not wgpu_start.shown(win)
    assert any(g.billboard for g in win.viewport.scene.groups)
    assert not win._is_dirty()


def test_an_external_edit_reveals_the_drawing(win):
    from ingetrazo_core import Vec3
    from core.history import AddEdgeCommand
    win.close_document()
    win.viewport.notify_scene_changed()
    wgpu_start.before_frame(win._host.view, win)
    assert wgpu_start.shown(win)
    win.viewport.history.execute(AddEdgeCommand(Vec3(), Vec3(1, 0, 0)))
    win.viewport.notify_scene_changed()
    wgpu_start.before_frame(win._host.view, win)
    assert not wgpu_start.shown(win)
    assert win._is_dirty()


def test_a_workspace_entered_from_start_is_visible(win):
    from core.history import History
    from core.scene import Scene

    class Workspace:
        allowed_tools = None

        def __init__(self):
            self.scene = Scene()
            self.history = History(self.scene)

        def title(self):
            return "w"

        def is_dirty(self):
            return False

        def confirm_leave(self):
            return True

    win.close_document()
    workspace = Workspace()
    assert win.enter_workspace(workspace)
    assert not wgpu_start.shown(win)
    assert win.viewport.scene is workspace.scene
    assert win.leave_workspace()
