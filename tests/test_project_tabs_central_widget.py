# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Document ownership of extension viewports and native split layouts (#1066)."""
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views.extension_api import ExtensionApp


def test_extension_sees_its_active_document_view(native_host):
    win = native_host.window
    app = ExtensionApp(win, "grid")
    first = win.active_document()
    assert app.viewport is first.viewport and app.viewport_widget is first.stand_in
    win.set_view_layout("four")
    second = win.new_document()
    assert app.viewport is second.viewport and app.viewport_widget is second.stand_in


def test_a_grid_of_views_keeps_its_document_and_tabs(native_host):
    win = native_host.window
    doc = win.active_document()
    scene, history = doc.viewport.scene, doc.viewport.history
    for layout in ("four", "single", "four", "single"):
        win.set_view_layout(layout)
        assert doc.view_layout == layout and win.documents() == [doc]
        assert win.viewport.scene is scene and win.viewport.history is history
        assert not win._is_dirty()


def test_switching_tabs_with_the_grid_on(native_host):
    win = native_host.window
    first = win.active_document()
    win.set_view_layout("four")
    second = win.new_document()
    assert win.active_document() is second and second.view_layout == "single"
    win.activate_document(first)
    assert first.view_layout == "four"
    win.activate_document(second)
    first.model_views.choose(first, "single")
    assert win.active_document() is second and win.view is second.stand_in
    assert first.view_layout == second.view_layout == "single"


def test_another_tab_keeps_its_own_extension_surface(native_host):
    win = native_host.window
    extension = ExtensionApp(win, "grid")
    saved = extension.viewport_widget
    second = win.new_document()
    assert extension.viewport_widget is second.stand_in and extension.viewport_widget is not saved
    assert win.documents()[0].stand_in is saved


def test_closing_a_split_project_releases_its_display(native_host):
    win = native_host.window
    first = win.active_document()
    win.set_view_layout("four")
    second = win.new_document()
    assert win.close_document(first, ask=False)
    assert first.stand_in is None and first.front_view is None and first.viewport is None
    assert win.documents() == [second] and win.view is second.stand_in
