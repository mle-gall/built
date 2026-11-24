# SPDX-License-Identifier: GPL-3.0-or-later
"""Row icons shared by the remaining Qt tree consumers.

The marks are an asset of the renderer (icon_art.rs, 18 px cells weighted
by the palette's text and base and a swatch's fill); this module only
wraps them in Qt types and caches them per palette."""
from functools import lru_cache

from ingetrazo_render import row_icon_art
from PySide6.QtGui import QColor, QIcon, QImage, QPixmap
from PySide6.QtWidgets import QApplication, QStyle

#: The «no colour» slash of the default layer's swatch.
NO_COLOUR_SLASH = QColor(214, 48, 49)


def _rgb(c):
    return (c.red(), c.green(), c.blue())


def _art(key, fill=(0, 0, 0)) -> QIcon:
    pal = QApplication.palette()
    w, h, rgba = row_icon_art(key, _rgb(pal.text().color()), fill, _rgb(pal.base().color()))
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


def swatch_icon(rgb) -> QIcon:
    fill = QColor.fromRgbF(*rgb[:3])
    return _art("swatch", _rgb(fill))


def layer_swatch(ly) -> QIcon:
    """A layer's swatch. The default layer with no colour picked has none
    of its own, and must not pass for one: the «none» swatch."""
    from core import layers as L
    if ly is None or ly.color is not None or ly.key != L.DEFAULT_LAYER:
        return swatch_icon(L.layer_color(ly))
    return none_swatch()


def none_swatch() -> QIcon:
    """The «none» swatch: empty (base-coloured), crossed by a red slash —
    a layer with no colour of its own, a layer with no hatch."""
    return _art("none_swatch")
