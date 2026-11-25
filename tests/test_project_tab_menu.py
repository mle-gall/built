# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A project tab's right-click menu: New, Open…, Save, Save As…, Close,
Close the ones to the right / to the left, Close all. The project right-clicked comes to the front first, so
Save and Save As act on it; each one closed asks only about itself."""
from __future__ import annotations

from ingetrazo_core import Vec3 as V

from ingetrazo_core.settings import settings
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import wgpu_tabs


def _three(win):
    a = win.active_document()
    b = win.new_document()
    c = win.new_document()
    win.activate_document(b)
    return a, b, c


def _menu(host, verb, index=1):
    wgpu_tabs.action(host.window, host.view, f"menu:{verb}:{index}")


def test_the_menu_turns_the_picture_on_hover_off_and_on(native_host):
    assert wgpu_tabs.pictures_on()
    for enabled in (False, True):
        _menu(native_host, "pictures", 0)
        assert wgpu_tabs.pictures_on() is enabled
        assert settings().value("tabs/hover_picture") == str(int(enabled))


def test_close_to_the_right(native_host):
    a, b, c = _three(native_host.window)
    _menu(native_host, "close_right")
    assert native_host.window.documents() == [a, b]


def test_close_to_the_left(native_host):
    a, b, c = _three(native_host.window)
    _menu(native_host, "close_left")
    assert native_host.window.documents() == [b, c]


def test_close_all_leaves_one_empty_project(native_host):
    old = _three(native_host.window)
    _menu(native_host, "close_all")
    blank, = native_host.window.documents()
    assert blank not in old and not native_host.window._is_dirty()


def test_a_cancel_stops_closing_the_rest(native_host, monkeypatch):
    win = native_host.window
    a, b, c = _three(win)
    win.activate_document(a)
    win.viewport.scene.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])
    win.viewport.scene.version += 1
    monkeypatch.setattr("ingetrazo_render.chrome_dialogs.ask", lambda *args, **kw: "cancel")
    _menu(native_host, "close_all", 2)
    assert win.documents() == [a, b, c]
    assert win.active_document() is a


def test_save_from_the_menu_saves_that_project(native_host, tmp_path, monkeypatch):
    from ingetrazo_render.shell_files import ShellFileDialog
    win = native_host.window
    a, b, c = _three(win)
    target = tmp_path / "from-the-menu.built"
    monkeypatch.setattr(ShellFileDialog, "getSaveFileName", lambda *args, **kw: (str(target), ""))
    _menu(native_host, "save", 2)
    assert target.is_file() and c.path == target
    assert win.active_document() is c and b.path is None


def test_a_workspace_prevents_a_menu_save_on_behalf_of_another_tab(native_host, tmp_path):
    win = native_host.window
    _, b, c = _three(win)
    b.path = tmp_path / "b.built"
    c.path = tmp_path / "c.built"
    win._workspace = object()
    try:
        _menu(native_host, "save", 2)
    finally:
        win._workspace = None
    assert win.active_document() is b
    assert not b.path.exists() and not c.path.exists()
