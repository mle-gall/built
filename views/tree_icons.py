# SPDX-License-Identifier: GPL-3.0-or-later
"""Row icons shared by the remaining Qt tree consumers."""
from functools import lru_cache
from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import QColor, QIcon, QPainter, QPen, QPixmap
from PySide6.QtWidgets import QApplication, QStyle

_ISOLATE_ICONS: dict = {}


def row_icon(key):
    return _row_icon(key, QApplication.palette().cacheKey())


@lru_cache(maxsize=48)
def _row_icon(key, palette_key):
    if key == "folder":
        return QApplication.style().standardIcon(QStyle.SP_DirIcon)
    ink = QApplication.palette().text().color()
    pm = QPixmap(18, 18)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    p.setPen(QPen(ink, 1.2))
    if key == "scene":
        p.drawRect(QRectF(2.5, 6.5, 9, 7))
        p.drawPolygon([QPointF(11.5, 9), QPointF(15.5, 6.5),
                       QPointF(15.5, 13.5)])
    elif key in ("lock", "lock_open"):
        # The shackle closed over the body, or swung up and open.
        p.drawRoundedRect(QRectF(4, 8.5, 10, 7.5), 1.5, 1.5)
        if key == "lock":
            p.drawArc(QRectF(5.5, 2.5, 7, 10), 0, 180 * 16)
            p.drawLine(QPointF(5.5, 7.5), QPointF(5.5, 8.5))
            p.drawLine(QPointF(12.5, 7.5), QPointF(12.5, 8.5))
        else:
            faint = QColor(ink)
            faint.setAlpha(110)
            p.setPen(QPen(faint, 1.2))
            p.drawRoundedRect(QRectF(4, 8.5, 10, 7.5), 1.5, 1.5)
            p.drawArc(QRectF(8.5, 1.5, 7, 9), 0, 180 * 16)
            p.drawLine(QPointF(8.5, 6), QPointF(8.5, 8.5))
    elif key in ("eye", "eye_off"):
        if key == "eye_off":
            p.setPen(QPen(QColor(ink.red(), ink.green(), ink.blue(), 110),
                          1.2))
        p.drawEllipse(QRectF(2, 5.5, 14, 7))
        p.setBrush(p.pen().color())
        p.drawEllipse(QRectF(7, 7, 4, 4))
        if key == "eye_off":
            p.setPen(QPen(ink, 1.4))
            p.drawLine(QPointF(3, 15), QPointF(15, 3))
    p.end()
    return QIcon(pm)


def isolate_icon(on: bool) -> QIcon:
    """A row's isolate mark: a target, filled on the layer shown alone.
    Drawn once per state and theme: the Browser paints one per row, and a
    document with ten thousand rows would otherwise draw ten thousand."""
    ink = QApplication.palette().text().color()
    key = (bool(on), ink.rgba())
    icon = _ISOLATE_ICONS.get(key)
    if icon is not None:
        return icon
    pm = QPixmap(18, 18)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    if on:
        p.setPen(QPen(ink, 1.3))
        p.drawEllipse(QRectF(2, 2, 14, 14))
        # Solid in the ink, not the accent: it must read on the list's
        # highlighted row too, which is the accent.
        p.setPen(Qt.NoPen)
        p.setBrush(ink)
        p.drawEllipse(QRectF(5, 5, 8, 8))
    else:
        faint = QColor(ink)
        faint.setAlpha(110)
        p.setPen(QPen(faint, 1.2))
        p.drawEllipse(QRectF(2, 2, 14, 14))
        p.drawEllipse(QRectF(6.5, 6.5, 5, 5))
    p.end()
    icon = QIcon(pm)
    _ISOLATE_ICONS[key] = icon
    return icon
