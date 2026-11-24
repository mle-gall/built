# SPDX-License-Identifier: GPL-3.0-or-later
"""Offset viewport bindings. The machine owns the source geometry and gesture."""

from ingetrazo_core import register_tool_adapter, register_tool_functions
from tools.overlay_painter import draw_overlay
from ingetrazo_core import Vec3 as QVector3D, ordered_edge_path as _run_from_edges, outline_normal as _newell

from core.i18n import tr
from core.mesh import Edge
from core.units import fmt_len
from tools.rust_tool_base import _point_property


def _field(key):
    return property(lambda self: self._state(key), lambda self, v: self._machine.set(key, v))


@register_tool_adapter
class OffsetHost:
    _READ_DURING = ("dragging", "distance", "ref_point", "ref_inward", "reference")
    hovered_face = hovered_edge = None
    _base_face = None
    dragging = _field("dragging")
    distance = _field("distance")
    _armed = _field("armed")
    _closed = _field("closed")
    _ref_point = _point_property("ref_point")
    _ref_inward = _point_property("ref_inward")
    allow_inferences = False
    wireframe_color = (0.13, 0.17, 0.23, 1.0)
    wireframe_depth_tested = True
    allowed_snap_kinds = frozenset((
        "endpoint", "midpoint", "arc_midpoint", "center", "origin",
        "component_origin", "intersection", "intersection_edge",
        "intersection_face",
    ))

    overlay_kind = "offset"
    draw_overlay = draw_overlay


register_tool_functions(globals())
