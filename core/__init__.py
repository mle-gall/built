# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""IngeTrazo core: configuration, scene graph, camera, geometry primitives, layers."""

# Keep the scripting import paths of the modules provided by the Rust extension.
# File and settings utilities can also be imported before the extension is built.
import sys as _sys

try:
    from ingetrazo_core import install_core_modules as _install_core_modules
    from ingetrazo_core import orient, saved_views, section, arrangement, desktop_open, camera, edits, hatches, history, mesh, push_scans, snap, snap_frame, snap_query, topology
except ModuleNotFoundError as _error:
    if _error.name != "ingetrazo_core":
        raise
else:
    _install_core_modules(__path__)
    for _plugin_module in (orient, saved_views, section):
        _sys.modules[__name__ + "." + _plugin_module.__name__.rsplit(".", 1)[1]] = _plugin_module
    _sys.modules[__name__ + ".desktop_open"] = desktop_open
    _sys.modules[__name__ + ".camera"] = camera
    _sys.modules[__name__ + ".arrangement"] = arrangement
    _sys.modules[__name__ + ".snap_query"] = snap_query
    _sys.modules[__name__ + ".push_scans"] = push_scans
    _sys.modules[__name__ + ".snap_frame"] = snap_frame
    _sys.modules[__name__ + ".topology"] = topology
    _sys.modules[__name__ + ".hatches"] = hatches
    _sys.modules[__name__ + ".edits"] = edits
    _sys.modules[__name__ + ".snap"] = snap
    _sys.modules[__name__ + ".history"] = history
    _sys.modules[__name__ + ".mesh"] = mesh
    store_mesh = _sys.modules[__name__ + ".store_mesh"]
    from ingetrazo_core import autosave, axes, geometry, guide, materials, paths, settings, structure, style, sun, textlabel, timers, triangulate, version
    for _module in (autosave, axes, geometry, guide, materials, paths, settings, structure, style, sun, textlabel, timers, triangulate, version):
        _sys.modules[__name__ + "." + _module.__name__.rsplit(".", 1)[1]] = _module
