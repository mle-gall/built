# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Recalling a scene that only moves the camera leaves ``scene.version``
alone: no edge/profile re-sync of the whole model, and the document is not
marked modified for a look around. A scene that changes what is shown
(layers, hidden objects…) still bumps it, so the caches refresh."""
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def _activate(win, name):
    scene = win.viewport.scene
    saved = next(view for view in scene.saved_views if view.name == name)
    win.tray.scenes.show_view(saved, animate=False)


def _window_with_two_scenes(native_host):
    from core.layers import Layer
    from ingetrazo_core.saved_views import SavedView

    win = native_host.window
    scene = win.viewport.scene
    scene.layers.append(Layer(name="Furniture"))
    cam = win.viewport.camera
    cam.yaw, cam.pitch, cam.distance = 0.3, 0.4, 30.0
    scene.saved_views.append(SavedView.capture("A", scene, cam))
    cam.yaw, cam.pitch, cam.distance = 1.2, 0.9, 12.0
    scene.saved_views.append(SavedView.capture("B", scene, cam))
    win._saved_version = scene.version
    return win, scene, cam


def test_camera_only_recall_keeps_version_and_clean_document(native_host):
    win, scene, cam = _window_with_two_scenes(native_host)
    v = scene.version
    _activate(win, "A")
    assert abs(cam.yaw - 0.3) < 1e-9 and abs(cam.distance - 30.0) < 1e-9
    _activate(win, "B")
    assert abs(cam.yaw - 1.2) < 1e-9 and abs(cam.distance - 12.0) < 1e-9
    assert scene.version == v
    assert not win._is_dirty()


def test_recall_that_changes_layers_still_bumps_version(native_host):
    from ingetrazo_core.saved_views import SavedView
    win, scene, cam = _window_with_two_scenes(native_host)
    furniture = next(ly for ly in scene.layers if ly.name == "Furniture")
    furniture.visible = False
    scene.saved_views.append(SavedView.capture("Plan", scene, cam))
    furniture.visible = True
    v = scene.version
    _activate(win, "Plan")
    assert not furniture.visible
    assert scene.version > v
