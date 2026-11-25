# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""IngeTrazo core: configuration, scene graph, camera, geometry primitives, layers."""

# Keep the scripting import paths of the modules provided by the Rust extension.
# File and settings utilities can also be imported before the extension is built.
import sys as _sys

try:
    from ingetrazo_core import arrangement, edits, hatches, push_scans, snap_frame, snap_query, topology
except ModuleNotFoundError as _error:
    if _error.name != "ingetrazo_core":
        raise
else:
    _sys.modules[__name__ + ".arrangement"] = arrangement
    _sys.modules[__name__ + ".snap_query"] = snap_query
    _sys.modules[__name__ + ".push_scans"] = push_scans
    _sys.modules[__name__ + ".snap_frame"] = snap_frame
    _sys.modules[__name__ + ".topology"] = topology
    _sys.modules[__name__ + ".hatches"] = hatches
    _sys.modules[__name__ + ".edits"] = edits
