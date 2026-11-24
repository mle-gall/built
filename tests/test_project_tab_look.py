# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Document actions from the native tab strip. Geometry is tested in ui/tabs.rs."""
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import wgpu_tabs


def tab_action(host, verb):
    wgpu_tabs.action(host.window, host.view, verb)


def test_the_cross_closes_its_project(native_host):
    win = native_host.window
    first = win.active_document()
    second = win.new_document()
    tab_action(native_host, "close:0")
    assert win.documents() == [second] and first.viewport is None


def test_plus_always_adds_a_tab(native_host):
    win = native_host.window
    for expected in (2, 3, 4):
        tab_action(native_host, "new")
        assert len(win.documents()) == expected
        assert win.active_document() is win.documents()[-1]
        assert not win._is_dirty()


def test_the_tab_menus_new_adds_a_tab_too(native_host):
    win = native_host.window
    original = win.active_document()
    tab_action(native_host, "menu:new:0")
    assert win.documents() == [original, win.active_document()]
    assert original is not win.active_document()
