# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The core protocol: commands, queries and a change feed over stable keys
(docs/core-protocol.md). The seam between the kernel and its consumers —
the Metal view's incremental update, the Outliner / Structure panels, the
agent bridge, the egui UI to come."""
from core.protocol.keys import (KINDS, element, elements, from_json_key,
                                known_mesh, mesh_key, resolve_mesh,
                                to_json_key)
from core.protocol.feed import ChangeFeed, MeshChange, Step

__all__ = ["KINDS", "ChangeFeed", "MeshChange", "Step", "element",
           "elements", "from_json_key", "known_mesh", "mesh_key",
           "resolve_mesh", "to_json_key"]
