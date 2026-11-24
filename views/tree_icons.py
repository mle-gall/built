# SPDX-License-Identifier: GPL-3.0-or-later
"""Row icons shared by the remaining Qt tree consumers.

The marks are an asset of the renderer (icon_art.rs, 18 px cells weighted
by the palette's text); this module only wraps them in Qt types and caches
them per palette."""
from functools import lru_cache

from ingetrazo_render import row_icon_art
from PySide6.QtGui import QIcon, QImage, QPixmap
from PySide6.QtWidgets import QApplication, QStyle


def _rgb(c):
    return (c.red(), c.green(), c.blue())


def _art(key) -> QIcon:
    pal = QApplication.palette()
    w, h, rgba = row_icon_art(key, _rgb(pal.text().color()), (0, 0, 0), _rgb(pal.base().color()))
    return QIcon(QPixmap.fromImage(QImage(rgba, w, h, w * 4, QImage.Format_RGBA8888_Premultiplied).copy()))


def row_icon(key):
    return _row_icon(key, QApplication.palette().cacheKey())


@lru_cache(maxsize=48)
def _row_icon(key, palette_key):
    if key == "folder":
        return QApplication.style().standardIcon(QStyle.SP_DirIcon)
    if key not in ("scene", "lock", "lock_open", "eye", "eye_off"):
        return QIcon()
    return _art(key)


def isolate_icon(on: bool) -> QIcon:
    """A row's isolate mark: a target, filled on the layer shown alone.
    Made once per state and theme: the Browser shows one per row."""
    return _row_icon("isolate_on" if on else "isolate_off", QApplication.palette().cacheKey())
