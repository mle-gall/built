# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""#1316: an .skp import that brought scenes crashed the app on macOS
right after it finished. ``_sync_edges`` runs inside ``paintGL`` and
announced the new version there; the Scenes panel followed and the scene
tab row came up under the viewport, resizing the view in the middle of
its own frame — the flush after it crashed. What a frame finds is now
announced once the frame is out, so no listener runs inside a paint."""
from __future__ import annotations

from ingetrazo_core import Vec3 as V

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views.extension_api import ExtensionApp


def test_notify_scene_changed_still_says_it_at_once(native_host):
    vp = native_host.viewport
    seen = []
    ExtensionApp(native_host.window, "probe").on_document_changed(lambda: seen.append(vp.scene.version))
    vp.scene.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
    vp.scene.version += 1
    vp.notify_scene_changed()
    assert seen == [vp.scene.version]
    native_host.idle()
    assert seen == [vp.scene.version]
