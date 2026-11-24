# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Viewport and extension bindings for the shared Rust protractor cursor."""
from __future__ import annotations

from ingetrazo_core import register_tool_adapter

from core.input_types import Qt
from ingetrazo_core import Vec3 as QVector3D

from core.guide import Guide
from ingetrazo_core import ToolDisc
from core.snap import COLOR_AXIS_X, COLOR_AXIS_Y, COLOR_AXIS_Z
from core.axes import plane_axes  # drawing axes (#44)
from tools.base import Tool, ToolContext

# The disc keeps a fixed SCREEN size; ticks every 15 degrees.
DISC_PX = 60.0
TICK_DEG = 15.0
# The drawing axes (core.axes): the open group's own inside it (#44).
from core.axes import AXES as _AXES  # noqa: E402
_AXIS_RGBA = {"x": (*COLOR_AXIS_X, 1.0), "y": (*COLOR_AXIS_Y, 1.0),
              "z": (*COLOR_AXIS_Z, 1.0)}
_OFF_AXIS_RGBA = (0.24, 0.27, 0.32, 1.0)


@register_tool_adapter
class ProtractorBase(ToolDisc, Tool):
    """Shared protractor state + behaviour (see module docstring)."""

    #: The Line tool's axis magnet for the arms (@pacaeiro, issue #41): a
    #: base arm within 3° of an axis that lies in the disc's plane lands
    #: on it. Only the world half — the screen detector would hand back a
    #: point on an axis OUTSIDE the disc's plane, which an arm cannot be.
    magnetic_axis_deg = 3.0
    #: The arms are directions from the centre, not lines being drawn: the
    #: snap engine leaves out 'through point', 'extension' and 'from point'
    #: for them, so the axis magnet is not outvoted (issue #140).
    radial_arm = True

    def __init__(self) -> None:
        self.start_point: QVector3D | None = None   # the protractor centre
        self.ref_point: QVector3D | None = None     # base (0°) direction
        self.hover_point: QVector3D | None = None

    def _infer_plane(self, ctx):
        from views.array_bake import reading_only
        with reading_only():
            self.infer_disc(ctx, bool(ctx.modifiers & Qt.ShiftModifier))


@register_tool_adapter
class ProtractorHost(ProtractorBase):
    accepts_angle_ratio = True
    _held_guide = None
