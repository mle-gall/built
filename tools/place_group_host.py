# SPDX-License-Identifier: GPL-3.0-or-later
"""Placement bounds, projected mesh previews and host selection handoff."""

from ingetrazo_core import register_tool_adapter
from ingetrazo_core import Vec3 as QVector3D, ToolMachine


@register_tool_adapter
class PlaceGroupHost:
    name = "Place component"
    uses_snap = True
    wireframe_color = (0.13, 0.17, 0.23, 1.0)
    wireframe_depth_tested = False      # the pending component floats on top

    def __init__(self, group, align_to_face: bool = False,
                 anchor: QVector3D | None = None) -> None:
        super().__init__()
        self._group = group
        from ingetrazo_core import placement_template
        self._anchor, self._segments = placement_template(group, align_to_face, anchor)
        self._align = align_to_face
        self._attachment = None
        self._machine.set("load", (group.uid, self._anchor.toTuple(), self._align, self._segments))
