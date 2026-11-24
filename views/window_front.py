# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Bring a Qt window to the front (moved out of views/composer.py)."""
from __future__ import annotations

from PySide6.QtCore import Qt


def bring_to_front(window) -> None:
    """Show ``window``, out of the taskbar if minimised, and raise it —
    ``show()`` alone leaves a minimised window where it is."""
    if window.isMinimized():
        window.setWindowState(window.windowState() & ~Qt.WindowMinimized)
    window.show()
    window.raise_()
    window.activateWindow()
