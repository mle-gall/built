# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""#759, second half: after File ▸ New the status bar kept showing the
previous model's UTM easting/northing — the viewport only clears the
readout on the next hover over it."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_file_new_clears_the_utm_readout(native_host):
    win = native_host.window
    from unittest.mock import Mock
    from ingetrazo_render import sync_status_readout
    win.viewport._last_coordinate_full = "E 211234.5  N 8212345.6  30N"
    win.viewport._last_coordinate = "E 211234.5  N 8212345.6"
    win._saved_version = win.viewport.scene.version   # nothing to discard
    win._on_new()
    win.view.renderer = Mock()
    sync_status_readout(win.view, "", "")
    win.view.renderer.ui_set_status.assert_called_once_with("", "", "", "")
    assert win.viewport._last_coordinate == ""
