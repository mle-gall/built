# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the solid tool gestures."""

from ingetrazo_core import register_tool_adapter
from tools.rust_tool_base import RustTool


_SOLID_ERRORS = {
    "not a solid": "That is not a solid — a group or component closed all "
                   "round, with nothing nested inside",
    "no overlap": "The two solids do not overlap",
    "empty": "The operation leaves nothing",
    "not manifold": "That solid has a flaw the operation cannot read",
    "two solids": "Pick two solids",
}


@register_tool_adapter
class SolidTool(RustTool):
    kind = "solid_union"
    shortcut = None

    def __init__(self):
        super().__init__()
        self._cache = {}
        self._cache_version = None
        self._viewport = None

    dragging = property(lambda self: self._state("first") is not None)
    _badge = property(lambda self: self._state("badge"))
    solid_cursor_key = property(lambda self: "solid-" + self._badge)
    op = property(lambda self: self.kind.removeprefix("solid_"))

class OuterShellTool(SolidTool):
    kind = "outer_shell"
    op = "outer_shell"
    shortcut = None


class UnionTool(SolidTool):
    kind = "solid_union"
    op = "union"
    shortcut = None


class SubtractTool(SolidTool):
    kind = "solid_subtract"
    op = "subtract"
    shortcut = None


class TrimTool(SolidTool):
    kind = "solid_trim"
    op = "trim"
    shortcut = None


class IntersectTool(SolidTool):
    kind = "solid_intersect"
    op = "intersect"
    shortcut = None


class SplitTool(SolidTool):
    kind = "solid_split"
    op = "split"
    shortcut = None


SOLID_TOOLS = (
    ("outer_shell", OuterShellTool), ("solid_union", UnionTool),
    ("solid_subtract", SubtractTool), ("solid_trim", TrimTool),
    ("solid_intersect", IntersectTool), ("solid_split", SplitTool),
)
