# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Preferences ▸ General ▸ Accent colour (#393): the icons' dots and the
toolbar hovers follow it; orange stays the default."""
from PySide6.QtGui import QColor
from PySide6.QtWidgets import QApplication, QToolButton

from views import theme

ORANGE = QColor(243, 115, 41)


def test_a_legacy_accent_cannot_change_the_chrome():
    from core import general_settings
    general_settings.set_value("accent", "orange")
    assert theme.accent_color().name() == "#6aa5ff"


def test_the_icons_and_the_toolbar_hover_share_the_accent():
    QApplication.instance() or QApplication([])
    button = QToolButton()
    theme.style(button, "QToolButton:hover {{ background: rgb({accent_rgb}); }}")
    assert "rgb(106, 165, 255)" in button.styleSheet()
    # The icons' accent: tests/test_icon_art.py.


def test_system_accent_selection_is_ignored():
    from core import general_settings
    general_settings.set_value("accent", "system")
    assert theme.accent_color() == QColor("#6aa5ff")
