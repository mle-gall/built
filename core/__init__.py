# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""IngeTrazo core: configuration, scene graph, camera, geometry primitives, layers."""

# Keep the scripting import paths of the modules provided by the Rust extension.
# File and settings utilities can also be imported before the extension is built.
import sys as _sys

try:
    from ingetrazo_core import arrangement
except ModuleNotFoundError as _error:
    if _error.name != "ingetrazo_core":
        raise
else:
    _sys.modules[__name__ + ".arrangement"] = arrangement
