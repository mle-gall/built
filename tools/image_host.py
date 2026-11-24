# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the image tool gestures."""
from tools.rust_tool_base import RustTool
from ingetrazo_core import Vec3
from tools.rust_tool_base import _point_property, _tuple


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

    @property
    def work_plane(self):
        return self._plane

    @work_plane.setter
    def work_plane(self, value):
        from core.axes import plane_axes
        self._plane = value
        self._machine.set("axes", [p.toTuple() for p in plane_axes(value[1].normalized())] if value else None)

    def load(self, path, aspect, label=""):
        self._plane = None
        self._machine.set("load", (str(path), float(aspect or 1), label or ""))

    def _send(self, vp, event):
        reply = super()._send(vp, event)
        if reply["committed"] or event["type"] in ("activate", "deactivate", "cancel"):
            self._plane = None
        if reply["committed"] and hasattr(vp, "finish_image_placement"):
            vp.finish_image_placement()
        return reply

    def _frame(self, a, b):
        return tuple(Vec3(*p) for p in self._machine.query("frame", [_tuple(a), _tuple(b)]))

    def value_label(self):
        from core.units import fmt_pair
        if self.armed and self.start_point is not None and self.hover_point is not None:
            u, v = self._frame(self.start_point, self.hover_point)
            return fmt_pair(u.length(), v.length()) + ("  (libre)" if self._free_aspect else ""), self.start_point + (u + v) * .5
