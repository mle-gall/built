# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Paste tool: drop a copied set of geometry, placing it with the cursor.

After Copy/Cut fills ``viewport.clipboard``, Paste activates this tool: the copied
faces, edges and groups follow the cursor as a live preview (snapping like any
draw), and a click stamps them into the scene ONCE and returns to Select.
The clipboard is kept — paste again for another copy.
"""
from __future__ import annotations
from ingetrazo_core import register_tool_adapter

from ingetrazo_core import Vec3 as QVector3D

from tools.base import Tool


@register_tool_adapter
class PasteHost(Tool):
    wireframe_color = (0.13, 0.17, 0.23, 1.0)
    wireframe_depth_tested = True
    _clipboard = None
    _preview_target = None
