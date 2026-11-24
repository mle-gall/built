# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The shared panel icon assets and the generated letter icon."""
from PySide6.QtGui import QIcon
from PySide6.QtWidgets import QApplication

_app = QApplication.instance() or QApplication([])

PANEL_KEYS = ("panel_properties", "panel_bim", "panel_terrain",
              "panel_browser", "panel_ai", "panel_render",
              "panel_solid_inspector", "panel_levels", "layers_panel")


def _image(icon: QIcon):
    return icon.pixmap(48, 48).toImage()


def test_every_panel_icon_is_drawn_and_its_own():
    from views.icons import tool_icon
    images = []
    for key in PANEL_KEYS:
        icon = tool_icon(key)
        assert not icon.isNull(), key
        img = _image(icon)
        assert any(img.pixelColor(x, y).alpha() > 0
                   for x in range(0, 48, 2) for y in range(0, 48, 2)), key
        images.append(img)
    for i, a in enumerate(images):
        for b in images[i + 1:]:
            assert a != b


def test_the_initial_is_drawn_with_the_weight_of_the_other_icons():
    """Its frame as thin as theirs: fewer inked pixels than at full weight."""
    from views import icons

    def inked(icon):
        img = _image(icon)
        return sum(1 for x in range(img.width()) for y in range(img.height())
                   if img.pixelColor(x, y).alpha() > 60)
    thin = inked(icons.letter_icon("Zeta"))
    stroke = icons.ICON_STROKE
    icons.ICON_STROKE = 1.0
    try:
        full = inked(icons.letter_icon("Zeta"))
    finally:
        icons.ICON_STROKE = stroke
    assert thin < full
