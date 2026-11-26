# SPDX-License-Identifier: GPL-3.0-or-later
"""Started by Built's "Open in Blender": ``blender [file.blend] --python launch.py``.

Uses the installed Built Link add-on when there is one, else this copy,
and links at once with the pairing code Built put in ``BUILT_LINK``.
"""
import os
import sys

import bpy


def _start():
    pairing = os.environ.get("BUILT_LINK", "")
    if not pairing:
        return None
    name = "built_link"
    if name not in bpy.context.preferences.addons:
        sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
        import built_link
        built_link.register()
        module = built_link
    else:
        module = sys.modules[name]
    from importlib import import_module
    ui = import_module(module.__name__ + ".ui")
    ui.link(pairing)
    return None


bpy.app.timers.register(_start, first_interval=0.5)
