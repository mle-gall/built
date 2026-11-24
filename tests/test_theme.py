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


def test_the_active_tool_is_only_a_thin_outline(app):
    """Round 12: the active tool is a thin quiet outline with nothing
    inside, as IngeTrazo always had it (a fill, round 7, was too loud)."""
    from views.icons import tool_icon
    win = QMainWindow()
    tb = QToolBar(win)
    tb.setIconSize(QSize(24, 24))
    win.addToolBar(tb)
    act = tb.addAction(tool_icon("select"), "")
    act.setCheckable(True)
    act.setChecked(True)
    win.resize(300, 120)
    win.show()
    app.processEvents()
    btn = tb.widgetForAction(act)
    img = btn.grab().toImage()
    bar = tb.grab().toImage()
    close_window(win)
    outline = theme.active_tool_colour()
    mid_y = img.height() // 2
    edge = img.pixelColor(0, mid_y)
    assert abs(edge.lightness() - outline.lightness()) <= 12, edge.name()
    # Inside, clear of the icon: the bar's own background, no fill.
    inside = img.pixelColor(3, mid_y)
    background = bar.pixelColor(bar.width() - 2, bar.height() // 2)
    # (Transparent where the stylesheet leaves the bar to show through;
    # the bar's gradient where Fusion paints it.)
    assert inside.alpha() < 10 or abs(
        inside.lightness() - background.lightness()) <= 8, (inside.name(), background.name())


def test_icons_are_inked_in_a_soft_grey(app):
    """Round 7: the icons' lines were the text colour, near white or near
    black; now a very light or very dark grey, a step softer."""
    from views.icons import _ink
    ink = _ink()
    assert ink == theme.icon_ink()
    text = app.palette().color(QPalette.WindowText)
    window = app.palette().color(QPalette.Window)
    # Softer than the text, still far from the background.
    assert abs(ink.lightness() - window.lightness()) \
        < abs(text.lightness() - window.lightness())
    assert abs(ink.lightness() - window.lightness()) > 100
    assert 25 < ink.lightness() < 230


def test_cursors_keep_the_full_contrast_ink(app):
    from views.icons import _text_ink
    assert _text_ink() == app.palette().color(QPalette.WindowText)


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


def _ink_pixels(icon, size=48) -> int:
    img = icon.pixmap(size, size).toImage()
    return sum(1 for x in range(img.width()) for y in range(img.height())
               if img.pixelColor(x, y).alpha() > 128)


def test_toolbar_icons_are_drawn_with_fine_lines(app):
    """Round 14: the icons' lines read heavy (1.5-2 px at 24 px); they are
    drawn at 65 % of their written width. The atlas keeps the thinned
    drawings: at full weight Line inked 152 or more pixels at 48 px."""
    from views import icons
    assert _ink_pixels(icons.tool_icon("line")) <= 130
    assert _ink_pixels(icons.tool_icon("move")) <= 215


def test_the_overflow_chevron_keeps_its_weight(app):
    # Thinned, the 16 px chevron faded to a grey blank again.
    from views import icons
    img = icons.tool_icon("overflow_h").pixmap(48, 48).toImage()
    # A full-weight stroke: some column holds three opaque pixels in a run.
    runs = [sum(1 for y in range(48) if img.pixelColor(x, y).alpha() > 200) for x in range(48)]
    assert max(runs) >= 3
