# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the north tool gestures."""

from ingetrazo_core import register_tool_adapter, register_tool_functions
from tools.rust_tool_base import RustTool
from ingetrazo_core import ToolMachine, Vec3
from tools.rust_tool_base import _tuple


COMPASS_PX = 70.0


@register_tool_adapter
class NorthTool(RustTool):
    kind = icon = "north"
    wireframe_color = (0.80, 0.16, 0.14, 1.0)
    centre = property(lambda self: self._point("centre"))
    hover_point = property(lambda self: self._point("hover_point"))
    _radius = property(lambda self: self._state("radius"))


register_tool_functions(globals())
