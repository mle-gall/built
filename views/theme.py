"""Fixed dark palette and Fusion drawing style for the remaining Qt widgets."""
from __future__ import annotations

from PySide6.QtCore import QRect, QRectF, Qt
from PySide6.QtGui import QColor, QPainter, QPalette, QPen
from PySide6.QtWidgets import QProxyStyle, QStyle

_HIGHLIGHT = QColor(42, 93, 143)


def brand_tint(dark: bool) -> QColor:
    """The IngeTrazo blue as a translucent wash over a see-through chrome
    (views.mac_glass). On the dark theme a deep, nearly black navy of that
    blue: the blue itself lightened the chrome to a slate, and a lighter
    navy still read too light (Maxence, 07-11). On the light theme only a
    trace of it — the light chrome stays neutral."""
    if dark:
        return QColor(3, 10, 22, 175)
    c = QColor(_HIGHLIGHT)
    c.setAlphaF(0.035)
    return c


def glass_tint(dark: bool) -> QColor:
    """The tint of the Liquid Glass panels (views.mac_glass): on the dark
    theme the chrome's navy, so the panels stay as dark as the bar under
    them; none on the light one."""
    return QColor(6, 14, 28, 150) if dark else QColor(0, 0, 0, 0)


# The desktop's accent, read before startup replaces the palette with
# IngeTrazo's own (Windows 11, macOS and the portals hand it to Qt there).
_system_accent: QColor | None = None


def accent_color() -> QColor:
    """The shared interface accent (ingetrazo_render/ui/tokens.rs)."""
    return QColor("#6aa5ff")


def _remember_system_accent(app) -> None:
    global _system_accent
    if _system_accent is None:
        role = getattr(QPalette.ColorRole, "Accent", QPalette.Highlight)
        _system_accent = QColor(app.palette().color(role))


def dark_palette() -> QPalette:
    # The Qt shell uses its historic blue-grey Fusion palette.
    window = QColor(0x22, 0x29, 0x33)
    text = QColor(0xdd, 0xe2, 0xe8)
    disabled = QColor(0x6c, 0x75, 0x81)
    p = QPalette()
    p.setColor(QPalette.Window, window)
    p.setColor(QPalette.WindowText, text)
    p.setColor(QPalette.Base, QColor(0x1b, 0x20, 0x28))
    p.setColor(QPalette.AlternateBase, window)
    p.setColor(QPalette.Text, text)
    p.setColor(QPalette.PlaceholderText, disabled)
    p.setColor(QPalette.Button, QColor(0x29, 0x30, 0x3a))
    p.setColor(QPalette.ButtonText, text)
    p.setColor(QPalette.BrightText, QColor(255, 104, 104))
    p.setColor(QPalette.ToolTipBase, QColor(0x33, 0x3a, 0x45))
    p.setColor(QPalette.ToolTipText, QColor(0xf0, 0xf3, 0xf7))
    p.setColor(QPalette.Highlight, _HIGHLIGHT)
    p.setColor(QPalette.HighlightedText, QColor(255, 255, 255))
    p.setColor(QPalette.Link, QColor(96, 166, 226))
    # The shades Fusion draws bevels and frames with — pinned so the look
    # does not move with the platform theme.
    p.setColor(QPalette.Light, QColor(0x34, 0x3a, 0x44))
    p.setColor(QPalette.Midlight, QColor(0x29, 0x30, 0x3a))
    p.setColor(QPalette.Mid, QColor(0x29, 0x30, 0x3a))
    p.setColor(QPalette.Dark, QColor(0x1b, 0x20, 0x28))
    p.setColor(QPalette.Shadow, QColor(0x0a, 0x0c, 0x0f))
    for role in (QPalette.WindowText, QPalette.Text, QPalette.ButtonText,
                 QPalette.HighlightedText):
        p.setColor(QPalette.Disabled, role, disabled)
    return p


def active_tool_colour() -> QColor:
    return QColor("#5b6676")


def icon_ink() -> QColor:
    return QColor("#d9dce1")


def grip_colour() -> QColor:
    return QColor("#485462")


def _in_toolbar(widget) -> bool:
    from PySide6.QtWidgets import QToolBar, QToolButton
    return (isinstance(widget, QToolButton)
            and isinstance(widget.parentWidget(), QToolBar))


class ChromeStyle(QProxyStyle):
    """Fusion, with the toolbars' grip drawn as one thin grey line instead
    of two columns of dots — narrower and quieter, in every theme (the
    user's choice). The floating toolbar uses this style too."""

    #: Room the grip takes along the bar (Fusion's dots take 9 px).
    GRIP_EXTENT = 7
    #: Gap between the line's ends and the bar's edges.
    GRIP_INSET = 6

    def pixelMetric(self, metric, option=None, widget=None):
        if metric == QStyle.PM_ToolBarHandleExtent:
            return self.GRIP_EXTENT
        return super().pixelMetric(metric, option, widget)

    #: Corner radius of the active tool's outline.
    ACTIVE_RADIUS = 3

    def drawPrimitive(self, element, option, painter, widget=None):
        if (element == QStyle.PE_PanelButtonTool
                and option.state & QStyle.State_On
                and _in_toolbar(widget)):
            # The active (checked) tool: a thin outline, nothing inside.
            painter.save()
            painter.setRenderHint(QPainter.Antialiasing, True)
            painter.setPen(QPen(active_tool_colour(), 1.0))
            painter.setBrush(Qt.NoBrush)
            painter.drawRoundedRect(QRectF(option.rect).adjusted(
                0.5, 0.5, -0.5, -0.5), self.ACTIVE_RADIUS, self.ACTIVE_RADIUS)
            painter.restore()
            return
        if element == QStyle.PE_IndicatorToolBarSeparator:
            # Between groups of tools: the grip's thin grey line, so every
            # line on a bar is one and the same.
            r, inset = option.rect, self.GRIP_INSET
            if option.state & QStyle.State_Horizontal:
                line = QRect(r.center().x(), r.top() + inset,
                             1, max(1, r.height() - 2 * inset))
            else:
                line = QRect(r.left() + inset, r.center().y(),
                             max(1, r.width() - 2 * inset), 1)
            painter.fillRect(line, grip_colour())
            return
        if element != QStyle.PE_IndicatorToolBarHandle:
            super().drawPrimitive(element, option, painter, widget)
            return
        r, inset = option.rect, self.GRIP_INSET
        if option.state & QStyle.State_Horizontal:   # a horizontal bar
            line = QRect(r.center().x(), r.top() + inset,
                         1, max(1, r.height() - 2 * inset))
        else:
            line = QRect(r.left() + inset, r.center().y(),
                         max(1, r.width() - 2 * inset), 1)
        painter.fillRect(line, grip_colour())


def tokens() -> dict[str, str]:
    """Colours a stylesheet template may name, for the palette in force."""
    accent = accent_color()
    return {"muted": "#8a94a0",
            # The text colour as an rgb triple, for veils: rgba({ink_rgb}, 30).
            "ink_rgb": "255, 255, 255",
            "tab_close": _glyph("close", True),
            "combo_arrow": _glyph("updown", True),
            "spin_up": _glyph("up", True),
            "spin_down": _glyph("down", True),
            # The desktop's own accent: what macOS menus highlight with.
            "system_accent_rgb": _rgb(_system_accent
                                      if _system_accent is not None
                                      and _system_accent.isValid()
                                      else _HIGHLIGHT),
            "accent_rgb": f"{accent.red()}, {accent.green()}, "
                          f"{accent.blue()}"}


def _rgb(c: QColor) -> str:
    return f"{c.red()}, {c.green()}, {c.blue()}"


def _glyph(kind: str, dark: bool) -> str:
    """A glyph of the glass chrome (views.mac_glass), as a path a
    stylesheet can name; empty where that chrome is not in use."""
    from views import mac_glass
    return mac_glass.glyph(kind, dark) if mac_glass.enabled() else ""


def style(widget, template: str) -> None:
    """Apply this widget's local stylesheet without taking ownership of it."""
    widget.setStyleSheet(template.format(**tokens()))


def install_dark_palette(app) -> None:
    """Install the Qt host style once, before constructing windows."""
    if getattr(app, "_built_chrome_style", None) is not None:
        return
    _remember_system_accent(app)
    app.styleHints().setColorScheme(Qt.ColorScheme.Dark)
    app._built_chrome_style = ChromeStyle("Fusion")
    app.setStyle(app._built_chrome_style)
    app.setPalette(dark_palette())
