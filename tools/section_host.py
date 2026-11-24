# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the section tool gestures."""

from ingetrazo_core import register_tool_adapter
from tools.rust_tool_base import RustTool
from tools.rust_tool_base import _point_property, _mods


@register_tool_adapter
class SectionPlaneTool(RustTool):
    kind = "section"
    shortcut = None
    hover_point = property(lambda self: self._point("hover_point"))
    _normal = _point_property("normal")
    _axis_pick = property(lambda self: self._state("axis"))
