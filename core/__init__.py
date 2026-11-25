# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Built core: configuration, scene graph, camera, geometry primitives, layers."""

# Native modules exposed under their original scripting or pickle paths.
import sys as _sys

try:
    from built_core import install_core_modules as _install_core_modules
    from built_core import (
        arrangement, axes, camera, edits, geometry, guide, hatches, history, i18n,
        materials, mesh, orient, push_scans, saved_views, section, snap,
        snap_frame, snap_query, structure, textlabel, topology,
    )
except ModuleNotFoundError as _error:
    if _error.name != "built_core":
        raise
else:
    _install_core_modules(__path__)
    for _module in (
        arrangement, axes, camera, edits, geometry, guide, hatches, history, i18n,
        materials, mesh, orient, push_scans, saved_views, section, snap,
        snap_frame, snap_query, structure, textlabel, topology,
    ):
        _sys.modules[f"{__name__}.{_module.__name__.rsplit('.', 1)[-1]}"] = _module
    store_mesh = _sys.modules[__name__ + ".store_mesh"]
