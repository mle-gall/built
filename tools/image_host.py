# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the image tool gestures."""

from ingetrazo_core import register_tool_adapter
from tools.rust_tool_base import RustTool
from ingetrazo_core import Vec3
from tools.rust_tool_base import _point_property, _tuple


@register_tool_adapter
class ImageTool(RustTool):
    kind = "image"
    shortcut = description = None
    MIN_SIZE = 1e-4
    diagonal_span = True
    chain_first_point = None
    _plane = None
    path = property(lambda self: self._state("path"))
    label = property(lambda self: self._state("label"))
    aspect = property(lambda self: self._state("aspect"))
    armed = property(lambda self: bool(self.path))
    start_point = _point_property("start_point")
    hover_point = _point_property("hover_point")
    _free_aspect = property(lambda self: self._state("free"),
                            lambda self, v: self._machine.set("free", bool(v)))
