# SPDX-License-Identifier: GPL-3.0-or-later
"""Qt painter and menu services for the Classic host."""

def draw_tile(tool, viewport, painter):
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


def texture_menu(tool, viewport, global_pos, rows):
    from PySide6.QtWidgets import QMenu
    from core.i18n import tr
    menu = QMenu(getattr(viewport, "keys_stand_in", None) or viewport)
    for group, items in rows:
        section = menu.addMenu(tr(group)) if group else menu
        for key, label, on in items:
            action = section.addAction(tr(label), lambda checked=False, key=key: tool.menu_action(viewport, key))
            action.setEnabled(on)
    menu.exec(global_pos)


def measure_view(scene):
    from PySide6.QtWidgets import QApplication
    from views.wgpu_view import WgpuView
    app = QApplication.instance() or QApplication([])
    view = WgpuView()
    view.resize(1000, 700)
    view.set_scene(scene)
    return app, view
