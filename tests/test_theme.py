# SPDX-License-Identifier: GPL-3.0-or-later
"""Drawing style, contrast and lifetime of the fixed dark Qt chrome."""
import weakref

import pytest
from PySide6.QtCore import QSize, Qt
from PySide6.QtGui import QColor, QPalette
from PySide6.QtWidgets import QApplication, QLabel, QMainWindow, QToolBar

from tests.qt_window_lifetime import close_window
from views import theme


@pytest.fixture
def app():
    return QApplication.instance()


def test_styling_does_not_own_the_widget():
    label = QLabel()
    theme.style(label, "color:{muted};")
    assert "{muted}" not in label.styleSheet()
    ref = weakref.ref(label)
    del label
    assert ref() is None


def test_dark_palette_defines_widget_shades():
    palette = theme.dark_palette()
    for role in (QPalette.Light, QPalette.Midlight, QPalette.Mid, QPalette.Dark, QPalette.Shadow):
        assert palette.isBrushSet(QPalette.Active, role)
    assert palette.windowText().color().lightness() > palette.window().color().lightness()


def test_palette_names_are_absent_from_settings():
    from tests.preferences_chrome import Controls
    from PySide6.QtWidgets import QWidget
    fields = str(Controls(QWidget(), "general").wire())
    assert "Solarized" not in fields and "Campbell" not in fields


def _grip_pixels(app, vertical=False):
    win = QMainWindow()
    tb = QToolBar(win)
    tb.setMovable(True)
    win.addToolBar(Qt.LeftToolBarArea if vertical else Qt.TopToolBarArea, tb)
    tb.addAction("a")
    win.resize(300, 300)
    win.show()
    app.processEvents()
    img = tb.grab().toImage()
    close_window(win)
    w, h = (img.width(), 12) if vertical else (12, img.height())
    return [img.pixelColor(x, y) for x in range(w) for y in range(h)]


@pytest.mark.parametrize("vertical", [False, True])
def test_the_toolbar_grip_is_one_grey_line(app, vertical):
    # Round 5: the one-line grip in every theme, its grey matched to each.
    grip = theme.grip_colour()
    px = _grip_pixels(app, vertical)
    line = [c for c in px if c == grip]
    assert len(line) >= 8           # a line, not a dot
    # ... and only one: the line is 1 px across.
    assert len(line) <= max(len(px) // 12 + 2, 40)


def test_the_grip_is_one_thin_line():
    # Round 3: one grey line instead of two columns of dots, to save width.
    from PySide6.QtWidgets import QStyle
    style = theme.ChromeStyle("Fusion")
    assert style.pixelMetric(QStyle.PM_ToolBarHandleExtent) == theme.ChromeStyle.GRIP_EXTENT < 9
    assert QApplication.instance().styleSheet() == ""


def test_retired_neutral_accent_is_ignored():
    from core import general_settings
    general_settings.set_value("accent", "light_grey")
    assert theme.accent_color().name() == "#6aa5ff"


def test_separators_are_the_grip_s_grey_line(app):
    """Round 10: the separators between groups of tools are the same thin
    grey line as the grip (Fusion drew a two-tone groove)."""
    win = QMainWindow()
    tb = QToolBar(win)
    tb.setMovable(False)
    win.addToolBar(tb)
    tb.addAction("a")
    sep = tb.addSeparator()
    tb.addAction("b")
    win.resize(300, 100)
    win.show()
    app.processEvents()
    geo = tb.actionGeometry(sep)
    img = tb.grab(geo).toImage()
    close_window(win)
    grip = theme.grip_colour()
    # The bar's background is a vertical gradient: a column that differs
    # from the first one is a line. Exactly one, in the grip's grey.
    def column(x):
        return [img.pixelColor(x, y).rgb() for y in range(img.height())]
    lines = [x for x in range(img.width()) if column(x) != column(0)]
    assert len(lines) == 1
    assert grip.rgb() in column(lines[0])


def _contrast(a: QColor, b: QColor) -> float:
    def lum(c):
        def f(v):
            v /= 255
            return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4
        return 0.2126 * f(c.red()) + 0.7152 * f(c.green()) + 0.0722 * f(c.blue())
    hi, lo = sorted((lum(a), lum(b)), reverse=True)
    return (hi + 0.05) / (lo + 0.05)


def test_dark_icons_have_a_reference_contrast():
    """Round 11: Dark's icons read dull (8.7:1) next to the user's
    reference toolbar (about 10.4:1); they are brighter now, still short
    of white."""
    window = theme.dark_palette().color(QPalette.Window)
    ink = theme.icon_ink()
    assert _contrast(ink, window) >= 10
    assert ink.lightness() < 240
    # Separators and the grip: visible but quiet, as in the reference.
    grip = theme.grip_colour()
    assert 1.6 <= _contrast(grip, window) <= 2.4


