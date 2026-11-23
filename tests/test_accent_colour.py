# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Preferences ▸ General ▸ Accent colour (#393): the icons' dots and the
toolbar hovers follow it; orange stays the default."""
from PySide6.QtGui import QColor
from PySide6.QtWidgets import QApplication, QToolButton

from views import theme
from views.icons import tool_icon

ORANGE = QColor(243, 115, 41)


def _has(icon, colour: QColor) -> bool:
    image = icon.pixmap(48, 48).toImage()
    for y in range(image.height()):
        for x in range(image.width()):
            c = image.pixelColor(x, y)
            if (c.alpha() > 200 and abs(c.red() - colour.red()) < 12
                    and abs(c.green() - colour.green()) < 12
                    and abs(c.blue() - colour.blue()) < 12):
                return True
    return False


def test_a_legacy_accent_cannot_change_the_chrome():
    from core import general_settings
    general_settings.set_value("accent", "orange")
    assert theme.accent_color().name() == "#6aa5ff"


def test_the_icons_and_the_toolbar_hover_share_the_accent():
    QApplication.instance() or QApplication([])
    button = QToolButton()
    theme.style(button, "QToolButton:hover {{ background: rgb({accent_rgb}); }}")
    assert "rgb(106, 165, 255)" in button.styleSheet()
    assert _has(tool_icon("line"), theme.accent_color())


def test_system_accent_selection_is_ignored():
    from core import general_settings
    general_settings.set_value("accent", "system")
    assert theme.accent_color() == QColor("#6aa5ff")
