# SPDX-License-Identifier: GPL-3.0-or-later
"""Several scenes deleted at once, in one undo step (#280).

The Scenes list took one scene at a time, and deleting one could not be
undone. Ctrl/Shift+click now picks several, − (or Delete) removes them
all, and Undo brings each back in its place."""
from __future__ import annotations

from built_core.saved_views import SavedView


from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_tree_links import _View
from views import wgpu_ui


def test_three_of_four_scenes_go_and_come_back(native_host):
    win = native_host.window
    scene = win.viewport.scene
    views = [SavedView.capture(f"S{i}", scene, win.viewport.camera) for i in range(4)]
    scene.saved_views[:] = views
    display = _View()
    def names():
        wgpu_ui._tabs(display, win)
        return [row[1] for row in display._ui_tabs[1]]
    assert names() == ["S0", "S1", "S2", "S3"]
    win.tray.scenes.delete_views([views[i] for i in (0, 2, 3)])
    assert names() == ["S1"]
    win._on_undo()
    assert names() == ["S0", "S1", "S2", "S3"]
    win._on_redo()
    assert names() == ["S1"]
