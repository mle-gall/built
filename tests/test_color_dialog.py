# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Every colour picker is Qt's own dialog: on GNOME the native one is a
small GTK palette, and opened from a right-click menu it came back without
the colour chosen, so editing a material's colour did nothing (0.5.7)."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from pathlib import Path  # noqa: E402

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from PySide6.QtGui import QColor  # noqa: E402
from PySide6.QtWidgets import QApplication, QColorDialog  # noqa: E402

_app = QApplication.instance() or QApplication([])
ROOT = Path(__file__).resolve().parents[1]


def test_the_picker_asks_for_qts_own_dialog(monkeypatch):
    from views import color_dialog
    seen = {}

    def fake(*args, **kwargs):
        seen.update(kwargs)
        return QColor(10, 20, 30)

    monkeypatch.setattr(color_dialog.QColorDialog, "getColor",
                        staticmethod(fake))
    assert color_dialog.get_color(QColor(0, 0, 0)).red() == 10
    assert seen["options"] == QColorDialog.ColorDialogOption.DontUseNativeDialog


def test_no_panel_opens_the_desktops_picker_directly():
    for rel in ("views", "plugins", "tools"):
        for path in (ROOT / rel).rglob("*.py"):
            if path.name == "color_dialog.py":
                continue
            assert "QColorDialog.getColor(" not in path.read_text("utf-8"), path


def test_a_temporary_message_gives_the_tool_hint_back(native_host):
    win = native_host.window
    win._activate_tool("line")
    bar = win.statusBar()
    standing = bar.currentMessage()
    assert standing.startswith("Line")
    win.viewport.flash_status("Temporary message")
    assert bar.currentMessage() == "Temporary message"
    bar.clearMessage()
    assert bar.currentMessage() == standing
