# SPDX-License-Identifier: GPL-3.0-or-later
"""ToolMachine event conversion, viewport queries and protocol dispatch."""
from __future__ import annotations

from ingetrazo_core import ToolBounds, ToolMachine, ToolHandles, Vec3, ViewportToolHost

from tools.base import Tool

selection_bounds = ToolBounds.selection

def _mods(modifiers) -> int:
    return int(getattr(modifiers, "value", modifiers) or 0)


def selection_mode(modifiers) -> str:
    """How a click or a box with these modifiers joins the selection
    (the Rust rule: Shift toggles, Ctrl adds, Shift+Ctrl removes)."""
    from ingetrazo_core import select_mode
    return select_mode(_mods(modifiers))


def _tuple(p):
    return None if p is None else (p.toTuple() if hasattr(p, "toTuple") else tuple(p))


class _Host(ViewportToolHost):
    """The machine's ``ToolHost`` over a viewport (the Metal view's
    ``NativeTools``, the controller, or a stand-in with scene and history)."""

    def cycle_status(self, index, count, ident):
        from tools import select_host as select
        select.cycle_status(self.vp, index, count, self.handles.get(ident))

    def select(self, ids, mode, grow):
        from tools import select_host as select
        select.apply(self.vp, [self.handles.get(i) for i in ids], mode, grow)

    def box_select(self, rect, crossing, mode):
        from tools import select_host as select
        select.box_select(self.vp, rect, crossing, mode)

    def erase(self, selection_only):
        from tools import select_host as select
        if selection_only:
            return select.erase_entities(self.vp, list(self.vp.scene.selection))
        return select.delete_selection_or_hover(self.vp)

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

    def host_verb(self, verb, ident):
        from tools import select_host as select
        select.HOST_VERBS[verb](self.vp, self.handles.get(ident))


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

    # ---- events -------------------------------------------------------------
    def _send(self, viewport, event):
        # The host's callbacks may read the tool back (a repaint asks for
        # the grab point): they get the state as it was before the event.
        self._during = {key: self._state(key) for key in self._READ_DURING}
        try:
            return self._machine.event(event, _Host(viewport, self._handles, self))
        finally:
            self._during = None

    #: The fields a callback may read while the machine is busy.
    _READ_DURING = ("start_point", "hover_point")
    _during = None

    @staticmethod
    def _pointer(kind, ctx, t=0.0):
        screen = getattr(ctx, "screen", None)
        return {"type": kind, "px": (screen.x(), screen.y()) if screen is not None else (0.0, 0.0),
                "mods": _mods(getattr(ctx, "modifiers", 0)), "world": _tuple(getattr(ctx, "world", None)),
                "snap_kind": getattr(getattr(ctx, "snap", None), "kind", None), "t": t}

    def on_activate(self, viewport) -> None:
        self._send(viewport, {"type": "activate"})

    def on_deactivate(self, viewport) -> None:
        self._send(viewport, {"type": "deactivate"})
        self._handles.clear()

    def on_cancel(self, viewport) -> None:
        self._send(viewport, {"type": "cancel"})
        viewport.update()

    def on_click(self, ctx) -> None:
        self._send(ctx.viewport, self._pointer("click", ctx))
        ctx.viewport.update()

    def on_hover(self, ctx) -> None:
        self._send(ctx.viewport, self._pointer("hover", ctx))
        ctx.viewport.update()

    def on_value(self, viewport, value) -> bool:
        return self._send(viewport, {"type": "value", "value": value})["consumed"]

    def on_undo(self, viewport) -> bool:
        consumed = self._send(viewport, {"type": "undo"})["consumed"]
        if consumed:
            viewport.update()
        return consumed

    def busy(self) -> bool:
        return self._machine.busy()

    # ---- what the inference and the painters read -------------------------
    def _state(self, key):
        if self._during is not None:
            return self._during.get(key)
        # The inference reads several fields per pointer move: one state
        # snapshot per machine serial.
        serial, fields = self._seen
        if serial != self._machine.serial:
            fields = {}
            self._seen = (self._machine.serial, fields)
        try:
            return fields[key]
        except KeyError:
            value = fields[key] = self._machine.get(key)
            return value

    def _point(self, key):
        """The point as a Vec3 that stays the same object while the value
        does: the snap frame tells the chain's start from its first point
        by identity."""
        value = self._state(key)
        if value is None:
            return None
        held = self._held_points.get(key)
        if held is None or held[0] != value:
            held = self._held_points[key] = (value, Vec3(*value))
        return held[1]

    def rubber_band_lines(self):
        return self._machine.preview_lines()

    def preview_faces(self):
        from core.geometry import Face
        return [Face(outer, holes or None) for outer, *holes in self._machine.preview_surfaces()]

    def preview(self):
        return self._machine.preview()


def _point_property(key):
    def get(self):
        return self._point(key)

    def put(self, value):
        self._machine.set(key, _tuple(value))
    return property(get, put)
