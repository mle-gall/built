# SPDX-License-Identifier: GPL-3.0-or-later
"""Blender rendering, camera follow and light placement in the native dock."""
from built_core import render_blender as rb
from built_core.i18n import tr
from plugins._blender_panel import BlenderPanel, CameraSession, create_panel

RenderPanel = BlenderPanel
SyncSession = CameraSession


def setup(app):
    from built_core.plugin_panels import open_panel
    app.add_menu_action(tr("Render with Blender…"), lambda: open_panel(app.window, app.key))
