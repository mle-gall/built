# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Window title: an imported .skp/.dae/.obj shows ITS name until the model
is saved as .igz (user report: opening a .skp file left "Untitled")."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_title_shows_imported_name_until_igz_takes_over(native_host, tmp_path):
    win = native_host.window
    assert "Untitled" in native_host._title or "Sin título" in native_host._title

    win._import_name = "casa bueno.skp"     # what a .skp import sets
    win._update_title()
    assert "casa bueno.skp" in native_host._title

    win._current_path = tmp_path / "obra.igz"   # Save As wins over it
    win._update_title()
    assert "obra.igz" in native_host._title
    assert "casa bueno" not in native_host._title

    win._current_path = None                 # New clears the import name
    win._import_name = None
    win._update_title()
    assert "obra.igz" not in native_host._title
