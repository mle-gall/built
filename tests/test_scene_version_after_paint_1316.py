# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""#1316: an .skp import that brought scenes crashed the app on macOS
right after it finished. ``_sync_edges`` runs inside ``paintGL`` and
announced the new version there; the Scenes panel followed and the scene
tab row came up under the viewport, resizing the view in the middle of
its own frame — the flush after it crashed. What a frame finds is now
announced once the frame is out, so no listener runs inside a paint."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from ingetrazo_core import Vec3 as V
from PySide6.QtWidgets import QApplication  # noqa: E402

_app = QApplication.instance() or QApplication([])



def _window():
    from views.main_window import MainWindow
    win = MainWindow()
    win.resize(800, 600)
    vp = win.viewport
    vp.scene.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
    vp.scene.version += 1
    return win


def _close(win):
    win._saved_version = win.viewport.scene.version
    win.close()


def test_notify_scene_changed_still_says_it_at_once():
    win = _window()
    try:
        vp = win.viewport
        seen: list = []
        vp.sceneVersionChanged.connect(seen.append)
        vp.notify_scene_changed()
        assert seen == [vp.scene.version]
    finally:
        _close(win)
