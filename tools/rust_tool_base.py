# SPDX-License-Identifier: GPL-3.0-or-later
"""ToolMachine event conversion, viewport queries and protocol dispatch."""
from __future__ import annotations

from ingetrazo_core import register_tool_adapter, register_tool_functions

from ingetrazo_core import ToolBounds, ToolMachine, ToolHandles, Vec3, ViewportToolHost

from tools.base import Tool

selection_bounds = ToolBounds.selection


@register_tool_adapter
class _Host(ViewportToolHost):
    """The machine's ``ToolHost`` over a viewport (the Metal view's
    ``NativeTools``, the controller, or a stand-in with scene and history)."""

    def message(self, template, args):
        flash = getattr(self.vp, "flash_status", None)
        if callable(flash):
            from core.i18n import tr
            # A length the machine names («len:2.0») reads in the
            # document's unit.
            for key, value in args.items():
                if value.startswith("len:"):
                    from core.units import fmt_len
                    args[key] = fmt_len(float(value[4:]))
            flash(tr(template, **args) if args else tr(template))


@register_tool_adapter
class RustTool(Tool):
    """A tool whose state machine is Rust. Subclasses name the machine."""

    kind = ""
    handle_limit = 4096

    def __init__(self):
        self._machine = ToolMachine(self.kind)
        self._handles = ToolHandles(self.handle_limit)
        self._seen = (-1, None)
        self._held_points = {}

    def __init_subclass__(cls, **kw):
        super().__init_subclass__(**kw)
        if not cls.kind:
            return
        info = ToolMachine(cls.kind).info
        for name in ("name", "description", "uses_snap", "box_select",
                     "vcb_label", "axis_source", "axis_source_empty_only"):
            if name not in cls.__dict__:
                setattr(cls, name, info[name])
        if "shortcut" not in cls.__dict__:
            cls.shortcut = info["shortcut"]
        for name in ("magnetic_axis_deg", "screen_axis_px"):
            if info[name] is not None and name not in cls.__dict__:
                setattr(cls, name, info[name])

    #: The fields a callback may read while the machine is busy.
    _READ_DURING = ("start_point", "hover_point")
    _during = None

def _point_property(key):
    def get(self):
        return self._point(key)

    def put(self, value):
        self._machine.set(key, _tuple(value))
    return property(get, put)

register_tool_functions(globals())
