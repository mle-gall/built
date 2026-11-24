# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Resident bevel targets, preview mesh and panel bindings for the Fillet machine."""
from __future__ import annotations
from ingetrazo_core import register_tool_adapter

from core.input_types import Qt
from ingetrazo_core import Vec3 as QVector3D, bevel_nearest_end, bevel_selection, bevel_targets

from core.fillet import CIRCULAR, plan_fillet
from core.units import fmt_len_fine
from core.i18n import tr
from core.mesh import Edge
from core.vertex_bevel import plan_vertex_bevel
from tools.base import Tool


@register_tool_adapter
class FilletHost(Tool):
    icon = "fillet"
    wireframe_color = (0.13, 0.17, 0.23, 1.0)
    wireframe_depth_tested = True

    #: Last radius used — the next fillet starts from it (metres).
    radius: float = 0.10
    #: Arc segments of the strip.
    segments: int = 8
    #: Shape of the section — 0.5 is the circular arc (see core.fillet).
    profile: float = CIRCULAR
    #: How the size is read; Ctrl walks them in this order.
    measure: str = "radius"
    _MEASURES = (("radius", "Radius"), ("offset", "Distance"),
                 ("width", "Width"), ("depth", "Depth"))
    #: How a box corner closes: "round" (patch) or "cut" (flat).
    corners: str = "round"
    #: A drawn outline instead of the superellipse (core.bevel_profiles):
    #: a preset's name, the editor's control points (a tuple), or None.
    shape = None
    _SHAPES = (("steps", "Steps"), ("supports", "Round with supports"),
               ("cornice", "Cornice"), ("crown", "Crown moulding"))
    #: What is bevelled: "edges", or "vertices" (the corners alone, the
    #: size then running along their edges).
    affect: str = "edges"

    def __init__(self) -> None:
        self.edges: list = []
        self.hover_edge = None
        self.hover_vertex = None
        self.hover_point: QVector3D | None = None
        self._plan = None
        self._plan_key = None
        self._message: str | None = None
        self._viewport = None

    _nearest_end = staticmethod(bevel_nearest_end)
