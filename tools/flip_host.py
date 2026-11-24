# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Flip tool — Flip (mirror).

Flow: with a selection, three semi-transparent planes appear over it —
red, green and blue, one per axis. Hovering highlights a plane; ONE CLICK
flips the selection about it. The arrow keys pick a plane (Right = red,
Left = green, Up = blue); tapping Ctrl toggles COPY mode, which leaves the
original and creates the flipped duplicate.

Deferred (documented): dragging a plane to reposition it, Alt
parent-context axes, and the magenta custom plane from hovering a face.
"""
from __future__ import annotations

from ingetrazo_core import register_tool_adapter

from tools.overlay_painter import draw_overlay
from core.input_types import Qt
from ingetrazo_core import Vec3 as QVector3D

from ingetrazo_core import ToolMachine
from core.group import Group
from core.mesh import Edge, Face
from core.axes import AXES as _AXES
from core.i18n import tr
from tools.base import Tool, ctrl_clause


@register_tool_adapter
class FlipHost(Tool):
    overlay_kind = "flip"
    draw_overlay = draw_overlay
