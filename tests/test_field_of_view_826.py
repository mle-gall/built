# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""#826 (Alvaro Celis): the 3D view's lens could not be changed — 45° fixed —
while the composer frames already offered 10°–120°. Camera ▸ Field of
View… sets it; the document and its scenes keep it."""
from __future__ import annotations

from core.saved_views import SavedView
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_field_of_view_is_set_kept_and_reset(tmp_path):
    from ingetrazo_render import Host
    host = Host(0, 900, 600, 1., settings=str(tmp_path / "profile"))
    win = host.window
    win.new_document()
    try:
        cam = win.viewport.camera
        assert win._registry_actions["menu:verb:field_of_view"].text()
        cam.perspective = True
        eye = cam.eye()
        assert win.set_field_of_view(70.0)
        assert cam.fov_deg == 70.0
        assert (cam.eye() - eye).length() < 1e-6       # the eye stays put
        assert win.set_field_of_view(500.0) and cam.fov_deg == 120.0
        assert win.set_field_of_view(0.2) and cam.fov_deg == 1.0   # long lenses (533 mm ≈ 2.6°)
        win.set_field_of_view(70.0)
        # A scene remembers it.
        view = SavedView.capture("vista", win.viewport.scene, cam)
        assert view.to_dict()["fov"] == 70.0
        # The document keeps it.
        win._saved_version = win.viewport.scene.version
        path = tmp_path / "fov.built"
        host.menu(f"save_to:{path}")
        cam.fov_deg = 45.0
        win._saved_version = win.viewport.scene.version
        # A copy: the same file would only bring its tab back (#1066).
        import shutil
        shutil.copy(path, tmp_path / "copy.built")
        assert win.open_path(tmp_path / "copy.built")
        cam = win.viewport.camera
        assert cam.fov_deg == 70.0
        # Parallel has no angle: refused, nothing changes.
        cam.perspective = False
        assert not win.set_field_of_view(30.0)
        assert cam.fov_deg == 70.0
    finally:
        win._saved_version = win.viewport.scene.version
        host.close()
