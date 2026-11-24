# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the geopath tool gestures."""

from ingetrazo_core import register_tool_adapter
from tools.rust_tool_base import RustTool
from ingetrazo_core import Vec3


@register_tool_adapter
class GeoPathTool(RustTool):
    kind = "geopath"
    _saved_view = None
    work_plane = (Vec3(), Vec3(0, 0, 1))
    nodes = property(lambda self: [Vec3(*p) for p in self._state("nodes")])
    hover_point = property(lambda self: self._point("hover_point"))
    hover_elevation = property(lambda self: self._state("elevation"))
