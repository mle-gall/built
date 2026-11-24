# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Viewport picks, cursor and overlays for the Rust measuring tape."""

from ingetrazo_core import register_tool_adapter
from tools.overlay_painter import draw_overlay
from core.input_types import Qt
from ingetrazo_core import Vec3 as QVector3D

from core.guide import Guide
from core.i18n import tr
from core.units import fmt_len, fmt_len_fine
from tools.base import Tool


@register_tool_adapter
class TapeHost(Tool):
    _source = None
    work_plane = None
    chain_first_point = None
    _MODES = (("line", "Guides: guide lines"), ("point", "Guides: guide points"),
              ("measure", "Guides: off — measure only"))

    overlay_kind = "tape"
    draw_overlay = draw_overlay
