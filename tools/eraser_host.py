# SPDX-License-Identifier: GPL-3.0-or-later
"""Viewport picking and wireframe bindings for an Eraser stroke."""

from ingetrazo_core import register_tool_adapter
from core.input_types import Qt
from core.dimension import Dimension
from core.group import Group
from core.guide import Guide, is_guide
from core.mesh import Edge
from core.textlabel import TextLabel
from georef.geopath import GeoPath


@register_tool_adapter
class EraserHost:
    wireframe_color = (0.90, 0.20, 0.15, 1.0)
    _viewport = None

    # A long stroke retains every swept edge until its one-step commit.
    handle_limit = None

    _stroke = property(lambda self: self._state("stroke"),
                       lambda self, value: self._machine.set("stroke", value))
    _hide = property(lambda self: self._state("hide"),
                     lambda self, hide: self._machine.set("hide", bool(hide)))

    marked = property(lambda self: {self._handles.get(i) for i in self._state("marked")},
                      lambda self, items: self._machine.set("marked", [self._handles.id(e) for e in items]))
