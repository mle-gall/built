# SPDX-License-Identifier: GPL-3.0-or-later
"""Present the Rust tool layer on a host's Qt paint device."""


def draw_overlay(tool, viewport, painter):
    if hasattr(painter, "draw_tool"):
        return painter.draw_tool(tool.overlay_kind, tool, viewport)
    from ingetrazo_render import tool_overlay_tile
    from PySide6.QtCore import QPointF
    from PySide6.QtGui import QImage
    device = painter.device()
    dpr = device.devicePixelRatioF()
    tile = tool_overlay_tile(tool.overlay_kind, tool, viewport, device.width(), device.height(), dpr)
    if tile is not None:
        x, y, width, height, rgba = tile
        image = QImage(rgba, width, height, width * 4, QImage.Format_RGBA8888_Premultiplied)
        image.setDevicePixelRatio(dpr)
        painter.drawImage(QPointF(x / dpr, y / dpr), image)
