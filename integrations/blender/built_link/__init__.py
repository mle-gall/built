# SPDX-License-Identifier: GPL-3.0-or-later
"""Built Link: a live link between Built and Blender.

Built sends its model (geometry, materials, lights, scenes as cameras)
and keeps it up to date; Blender sends back changed materials, lights,
cameras and placement transforms, which Built reviews before applying.
Pair with the code from Built's Render drawer (Open in Blender links
automatically). The 3D Viewport sidebar has a Built tab.
"""
bl_info = {
    "name": "Built Link",
    "author": "Built contributors",
    "version": (1, 0, 0),
    "blender": (4, 2, 0),
    "location": "3D Viewport › Sidebar › Built",
    "description": "Live link with a Built model: geometry from Built, materials, lights, cameras and transforms back",
    "category": "Import-Export",
}


def register():
    from . import ui
    ui.register()


def unregister():
    from . import ui
    ui.unregister()
