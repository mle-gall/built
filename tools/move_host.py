# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Viewport bindings for the Rust Move tool, including live deformation and grips."""
from __future__ import annotations
from ingetrazo_core import register_tool_adapter, register_tool_functions

from tools.overlay_painter import draw_overlay
from core.input_types import Qt
from ingetrazo_core import Vec3 as QVector3D

from core.i18n import tr
from tools.base import ToolContext
from tools.rust_tool_base import RustTool, _mods, _point_property, _tuple
from core.units import fmt_len


from ingetrazo_core import (CopyWire, GripTurn, MoveGesture, TransformPose, moving_faces, rotation_handles as rotation_grips,
                            snap_exclusions, track_move_grips, transform_records, transform_targets as gather_targets)


class CopyPreview:
    def _gather_copy_entities(self, ctx):
        CopyWire.capture(self, ctx)

    def _fill_groups_ghost(self):
        self._base_segments.fill_groups()


#: How near (px) the cursor must come to a rotation grip to take it.
GRIP_PX = 9.0


@register_tool_adapter
class MoveHost(CopyPreview, RustTool):
    """Move (``rust/ingetrazo_core/src/tools/move_tool.rs`` decides: the
    grab, the Ctrl copy, typed distances and deltas, the array window).
    This class is the host: what a click grabs (it is picking), the live
    preview of the grabbed geometry, the rotation grips (#115, run by the
    Rotate tool) and the commit through ``core.move_ops``."""

    kind = ""
    _READ_DURING = ("grab", "hover_point", "copy", "window")
    # Drag on a camera-facing vertical plane (not the ground) so pulling the
    # mouse up raises geometry — what lets you lift a roof ridge straight up.
    prefers_vertical_drag = True
    accepts_array = True  # VCB "3x" / "/3" after a copy (arrays)

    def __init__(self) -> None:
        RustTool.__init__(self)
        self.chain_first_point = None  # silence close-snap
        self._ctx = None
        self._move_gesture = MoveGesture()
        self._clear_sets()
        # Rotation grips (issue #115): the group they sit on, the grips,
        # the one under the cursor, and — once one is taken — the Rotate
        # tool that runs the turn (its protractor, snaps, VCB and commit).
        self._grip_group = None
        self._grips: list = []
        self._hot_grip: int | None = None
        self._turn = GripTurn()

    _grip_rot = property(lambda self: self._turn.active)

    start_point = _point_property("grab")
    grab = _point_property("grab")
    hover_point = _point_property("hover_point")

    overlay_kind = "move"
    draw_overlay = draw_overlay


register_tool_functions(globals())
