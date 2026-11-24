# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Toolbar, menu and panel icons for the Classic (Qt) host.

The drawings are IngeTrazo's own, now an asset of the renderer
(``ingetrazo_render`` icon_art.rs): each 48 px cell keeps its ink, accent
and base weights, so the icons still follow the theme. This module only
wraps the recoloured pixels in Qt types. The native window never imports
it: its rail and cursors come from the renderer directly.

``tool_icon(key)`` returns a :class:`QIcon` for a tool/nav key, or a null
icon for an unknown key (the action keeps its text label).
"""
from __future__ import annotations

from ingetrazo_render import icon_art, icon_keys, letter_art
from PySide6.QtCore import QPointF, Qt
from PySide6.QtGui import QColor, QCursor, QIcon, QImage, QPixmap, QPolygonF
from PySide6.QtWidgets import QApplication

#: The keys the atlas draws (tool, view, panel and sheet icons).
KEYS = frozenset(icon_keys())
_PX = 48


def _ink() -> QColor:
    """Icon ink colour — the theme's icon grey: a very light grey on the
    dark themes and a very dark one on the light (views/theme.py)."""
    if QApplication.instance() is not None:
        from views.theme import icon_ink
        return icon_ink()
    return QColor(40, 44, 52)


def _text_ink() -> QColor:
    """The palette's text colour."""
    app = QApplication.instance()
    if app is not None:
        c = app.palette().windowText().color()
        return QColor(c.red(), c.green(), c.blue())
    return QColor(40, 44, 52)


def _accent() -> QColor:
    from views.theme import accent_color
    return accent_color()


def _base() -> QColor:
    return QApplication.palette().base().color() if QApplication.instance() else QColor(255, 255, 255)


def _rgb(c: QColor):
    return (c.red(), c.green(), c.blue())


def _icon(art) -> QIcon:
    if art is None:
        return QIcon()
    w, h, rgba = art
    image = QImage(rgba, w, h, w * 4, QImage.Format_RGBA8888_Premultiplied).copy()
    return QIcon(QPixmap.fromImage(image))


def tool_icon(key: str) -> QIcon:
    """The icon for a tool/nav ``key`` in the theme's colours (null if unknown)."""
    return _icon(icon_art(key, _rgb(_ink()), _rgb(_accent()), _rgb(_base())))


def letter_icon(name: str) -> QIcon:
    """A panel that brings no icon of its own: its initial in a rounded
    square, in the tools' ink, so its tab is never blank."""
    return _icon(letter_art(name, _rgb(_ink()), _rgb(_accent()), _rgb(_base())))


def pencil_glyph(p) -> None:
    """The layers' pencil in the 48 canvas (x 27–43.5, y 25.5–42), with
    the painter's pen and brush: the Layers tab's active-layer mark."""
    p.drawPolygon(QPolygonF([QPointF(27, 42), QPointF(28.5, 36),
                             QPointF(39, 25.5), QPointF(43.5, 30),
                             QPointF(33, 40.5)]))
    p.drawLine(QPointF(36.5, 28), QPointF(41, 32.5))


# ---- Tool cursors (the renderer's artwork, tool_cursors.rs) --------------------
_CURSOR_SIZE = 32   # logical cursor canvas (48-space hotspots scale onto it)
_cursor_cache: dict = {}


def clear_cursor_cache() -> None:
    """Forget the made cursors (the accent colour changed)."""
    _cursor_cache.clear()


def cursor_hotspot(hot48, scale: float, dpr: float,
                   platform: str | None = None) -> tuple[int, int]:
    """The hotspot to hand QCursor for a cursor drawn at ``dpr``.

    Windows and Wayland read a pixmap cursor's hotspot in logical pixels;
    X11 (xcb, also XWayland) takes it in the image's own pixels."""
    if platform is None:
        app = QApplication.instance()
        platform = app.platformName() if app is not None else ""
    k = dpr if platform == "xcb" else 1.0
    top = int(round(_CURSOR_SIZE * k)) - 1
    return (max(0, min(top, round(hot48[0] * scale * k))),
            max(0, min(top, round(hot48[1] * scale * k))))


def tool_cursor(key: str | None, plus: bool = False) -> QCursor | None:
    """A tool cursor, or ``None`` to keep the standard arrow (unknown keys,
    and Select). ``plus`` adds the Tape/Protractor guide-mode «+»."""
    from core import general_settings
    from ingetrazo_render import tool_cursor_name
    name = tool_cursor_name(key, plus, str(general_settings.value("cursor_style", "tool")))
    if name == "crosshair":
        return QCursor(Qt.CrossCursor)
    return _cursor_art(name[5:]) if name else None


def solid_cursor(state: str):
    """The Solid Tools pointer: a «×» badge over anything that is not a
    solid, a «1» or «2» over a solid for the pick it will be."""
    return _cursor_art("solid-" + state)


def _cursor_art(key):
    from ingetrazo_render import tool_cursor_art
    app = QApplication.instance()
    screen = app.primaryScreen() if app is not None else None
    dpr = screen.devicePixelRatio() if screen is not None else 1.0
    cache_key = (key, dpr)
    if cache_key not in _cursor_cache:
        side = max(1, min(256, round(_CURSOR_SIZE * dpr)))
        art = tool_cursor_art(key, side)
        if art is None:
            return None
        rgba, hotspot = art
        pm = QPixmap.fromImage(QImage(rgba, side, side, side * 4, QImage.Format_RGBA8888))
        pm.setDevicePixelRatio(dpr)
        _cursor_cache[cache_key] = QCursor(pm, *cursor_hotspot(hotspot, _CURSOR_SIZE / _PX, dpr))
    return _cursor_cache[cache_key]


# Fixed size for the remaining Qt toolbars. The native rail has its own metrics.
DEFAULT_TOOLBAR_ICON_PX = 24


def style_overflow_button(tb) -> None:
    """The «more tools» button a toolbar grows when its icons do not fit,
    a double chevron in the accent with a tooltip. Qt resets the button's
    icon on every orientation change, so it is reapplied then."""
    from PySide6.QtCore import QSize
    from PySide6.QtWidgets import QToolButton
    from core.i18n import tr
    btn = tb.findChild(QToolButton, "qt_toolbar_ext_button")
    if btn is None:
        return

    def apply(*_a):
        vertical = tb.orientation() == Qt.Vertical
        btn.setIcon(tool_icon("overflow_v" if vertical else "overflow_h"))
        btn.setIconSize(QSize(16, 16))
        btn.setToolTip(tr("More tools of this bar"))
        btn.setCursor(Qt.PointingHandCursor)
    apply()
    tb.orientationChanged.connect(apply)
    tb.iconSizeChanged.connect(apply)
