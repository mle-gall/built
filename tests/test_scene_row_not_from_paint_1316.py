# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Publishing new saved scenes never resizes the model inside a frame (#1316)."""
import pytest

from tests.scene_controls import rows
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_imported_scenes_reach_the_next_frame_without_resizing_the_view(native_host, monkeypatch):
    from core.saved_views import SavedView
    win = native_host.window
    display = win.view
    monkeypatch.setattr(display, "resizeEvent", lambda *_: pytest.fail("resized inside a frame"))
    assert not rows(win)
    display._drawing_frame = True
    try:
        scene = win.viewport.scene
        scene.saved_views.append(SavedView.capture("Imported", scene, win.viewport.camera))
        scene.version += 1
        win._refresh_scene_tabs()
        win._refresh_scene_tabs()
        assert display._ui_tabs is None
    finally:
        display._drawing_frame = False
    assert [row[1] for row in rows(win)] == ["Imported"]


def test_frame_guard_is_released_even_when_presentation_raises(native_host, monkeypatch):
    display = native_host.view
    seen = []

    def frame():
        seen.append(display._drawing_frame)
        raise RuntimeError("a broken frame")

    monkeypatch.setattr(display, "renderer", object())
    monkeypatch.setattr(display, "_present_frame", frame)
    display._resize_timer.stop()
    with pytest.raises(RuntimeError):
        display.draw_frame()
    assert seen == [True]
    assert not display._drawing_frame
