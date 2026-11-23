# SPDX-License-Identifier: GPL-3.0-or-later
"""The Rust tools seen from Python (docs/rust-tools.md §10).

Each class keeps the name, attributes and methods the hosts, the inference
and extensions use (``on_click(ctx)``, ``rubber_band_lines()``,
``start_point``…), turns the call into an event for its
``ingetrazo_core.ToolMachine`` and answers the machine's host calls. The
machines decide; this module only converts values and runs:

- edits through ``core.protocol.commands.run`` (the one front door);
- picks and the selection through ``tools.select``'s host queries, until
  picking is native.

Listed in docs/python-glue.md. Nothing here decides how a tool behaves.
"""
from __future__ import annotations

import json

from ingetrazo_core import ToolMachine, Vec3

from tools.base import Tool

_MASK = (1 << 63) - 1


def _mods(modifiers) -> int:
    return int(getattr(modifiers, "value", modifiers) or 0)


def selection_mode(modifiers) -> str:
    """How a click or a box with these modifiers joins the selection
    (the Rust rule: Shift toggles, Ctrl adds, Shift+Ctrl removes)."""
    from ingetrazo_core import select_mode
    return select_mode(_mods(modifiers))


def _tuple(p):
    return None if p is None else (p.toTuple() if hasattr(p, "toTuple") else tuple(p))


class _Host:
    """The machine's ``ToolHost`` over a viewport (the Metal view's
    ``NativeTools``, the controller, or a stand-in with scene and history)."""

    def __init__(self, viewport, handles=None):
        self.vp = viewport
        self.handles = handles

    # ---- commits: the protocol only ---------------------------------------
    def _frame_origin(self):
        st = self.vp.scene.mesh.__dict__.get("_store")
        origin = getattr(st, "frame_origin", None) if st is not None else None
        return tuple(origin) if origin is not None and any(origin) else None

    def _model(self, p):
        """A tool-frame point as the protocol takes it (the open mesh)."""
        origin = self._frame_origin()
        return list(p) if origin is None else [a + o for a, o in zip(p, origin)]

    def run(self, name, args):
        from core.protocol.commands import run
        args = json.loads(args)
        for key in ("a", "b", "eye"):
            if key in args:
                args[key] = self._model(args[key])
        history = self.vp.history
        before = self.revision()
        faces = len(self.vp.scene.faces)
        answer = run(history, name, **args)
        after = self.revision()
        return (after != before, after, len(self.vp.scene.faces) - faces,
                answer.get("error"))

    def revision(self):
        """The top of the undo stack: a retype may redo the command it
        made while this is unchanged (``tools.base.retype_open``)."""
        stack = getattr(self.vp.history, "undo_stack", None) or ()
        return hash((len(stack), id(stack[-1]) if stack else 0)) & _MASK

    def eye(self):
        cam = getattr(self.vp, "camera", None)
        return _tuple(cam.eye()) if cam is not None and hasattr(cam, "eye") else None

    # ---- picking and selection: tools.select's host queries ---------------
    def _id(self, entity):
        return self.handles.id(entity)

    def _pick(self, entity):
        if entity is None:
            return None
        from tools import select
        return self._id(entity), select.pick_kind(entity)

    def pick(self, x, y):
        from tools import select
        return self._pick(select.pick(self.vp, x, y))

    def candidates(self, x, y):
        from tools import select
        return [self._pick(e) for e in select.candidates(self.vp, x, y)]

    def extension_at(self, x, y):
        pick_item = getattr(self.vp, "pick_extension_item", None)
        hit = pick_item(x, y) if pick_item is not None else None
        return None if hit is None else self._id(hit)

    def cycle_status(self, index, count, ident):
        from tools import select
        select.cycle_status(self.vp, index, count, self.handles.get(ident))

    def select(self, ids, mode, grow):
        from tools import select
        select.apply(self.vp, [self.handles.get(i) for i in ids], mode, grow)

    def clear_selection(self):
        self.vp.scene.clear_selection()

    def selection_signature(self):
        return hash(frozenset(self.vp.scene.selection)) & _MASK

    def selection_empty(self):
        return not self.vp.scene.selection

    def box_select(self, rect, crossing, mode):
        from tools import select
        select.box_select(self.vp, rect, crossing, mode)

    def set_hover(self, ident):
        self.vp.set_hover(None if ident is None else self.handles.get(ident))

    def set_extension_pick(self, ident):
        if ident is None:
            self.vp.clear_extension_pick()
        else:
            self.vp.set_extension_pick(self.handles.get(ident))

    def has_extension_pick(self):
        return getattr(self.vp, "extension_pick", None) is not None

    def erase(self, selection_only):
        from tools import select
        if selection_only:
            return select.erase_entities(self.vp, list(self.vp.scene.selection))
        return select.delete_selection_or_hover(self.vp)

    def editing(self):
        return self.vp.scene.edit_group is not None

    def edit_box_contains(self, x, y):
        contains = getattr(self.vp, "edit_group_box_contains_pixel", None)
        return bool(callable(contains) and contains(x, y))

    def open_placement(self, ident):
        self.vp.begin_group_edit(self.handles.get(ident))

    def close_placement(self):
        self.vp.end_one_group_edit()

    def host_verb(self, verb, ident):
        from tools import select
        select.HOST_VERBS[verb](self.vp, self.handles.get(ident))


class _Handles:
    """Host objects the machine names by number: equal entities (two views
    of one face) share a number, so a repeated click sees the same picks."""

    LIMIT = 4096

    def __init__(self):
        self._ids = {}
        self._objects = {}

    def id(self, entity):
        try:
            key = ("v", entity)
            hash(key)
        except TypeError:
            key = ("i", id(entity))
        ident = self._ids.get(key)
        if ident is None:
            if len(self._ids) >= self.LIMIT:
                self._ids.clear()
                self._objects.clear()
            ident = self._ids[key] = len(self._ids) + 1
            self._objects[ident] = entity
        return ident

    def get(self, ident):
        return self._objects.get(ident)

    def clear(self):
        self._ids.clear()
        self._objects.clear()


class RustTool(Tool):
    """A tool whose state machine is Rust. Subclasses name the machine."""

    kind = ""

    def __init__(self):
        self._machine = ToolMachine(self.kind)
        self._handles = _Handles()
        self._seen = (-1, None)
        self._points = {}

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
        return self._machine.event(event, _Host(viewport, self._handles))

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
        held = self._points.get(key)
        if held is None or held[0] != value:
            held = self._points[key] = (value, Vec3(*value))
        return held[1]

    def rubber_band_lines(self):
        return [(Vec3(*a), Vec3(*b)) for a, b, _style in self._machine.preview()["lines"]]

    def preview(self):
        return self._machine.preview()


def _point_property(key):
    def get(self):
        return self._point(key)

    def put(self, value):
        self._machine.set(key, _tuple(value))
    return property(get, put)


class LineTool(RustTool):
    """Line (``rust/ingetrazo_core/src/tools/line.rs``)."""

    kind = "line"

    start_point = _point_property("start_point")
    hover_point = _point_property("hover_point")

    @property
    def chain_first_point(self):
        # One vertex placed: the first point IS the start (the snap frame
        # offers no ``close`` onto the segment's own start).
        if len(self._state("chain_vertices") or ()) <= 1:
            return self.start_point
        return self._point("chain_first_point")

    @chain_first_point.setter
    def chain_first_point(self, value):
        self._machine.set("chain_first_point", _tuple(value))

    @property
    def chain_vertices(self):
        return [Vec3(*p) for p in self._state("chain_vertices")]

    @chain_vertices.setter
    def chain_vertices(self, points):
        self._machine.set("chain_vertices", [_tuple(p) for p in points])

    @property
    def work_plane(self):
        plane = self._state("work_plane")
        return None if plane is None else (Vec3(*plane[0]), Vec3(*plane[1]))

    @work_plane.setter
    def work_plane(self, plane):
        self._machine.set("work_plane", None if plane is None else (_tuple(plane[0]), _tuple(plane[1])))

    @property
    def _last(self):
        """Truthy while a typed value would retype the segment just drawn."""
        return True if self._state("retype") else None


class SelectTool(RustTool):
    """Select (``rust/ingetrazo_core/src/tools/select.rs``). ``press``,
    ``drag``, ``release`` and ``double`` are the raw gesture the Metal view
    sends; the ``on_*`` methods are the older, already-decided entry points."""

    kind = "select"

    def _px(self, viewport, kind, x, y, modifiers, t=0.0):
        return self._send(viewport, {"type": kind, "px": (x, y), "mods": _mods(modifiers), "t": t})

    def hover_at(self, viewport, x, y, modifiers):
        return self._px(viewport, "hover", x, y, modifiers)

    def on_hover(self, ctx) -> None:
        self._send(ctx.viewport, self._pointer("hover", ctx))

    def pressed(self) -> bool:
        """A press is held that its release will answer (not a triple)."""
        return bool(self._state("pressed"))

    def press(self, viewport, x, y, modifiers, t):
        return self._px(viewport, "press", x, y, modifiers, t)

    def drag(self, viewport, x, y, modifiers):
        self._px(viewport, "drag", x, y, modifiers)
        return self._machine.preview()["box"]

    def release(self, viewport, x, y, modifiers):
        return self._px(viewport, "release", x, y, modifiers)

    def double(self, viewport, x, y, modifiers, t):
        return self._px(viewport, "double_click", x, y, modifiers, t)

    def on_double_click(self, ctx) -> None:
        self._send(ctx.viewport, self._pointer("double_click", ctx))
        ctx.viewport.update()

    def on_triple_click(self, ctx) -> None:
        self._send(ctx.viewport, self._pointer("triple_click", ctx))
        ctx.viewport.update()

    _MODE_MODS = {"replace": 0, "add": 0x04000000, "toggle": 0x02000000, "remove": 0x06000000}

    def on_box_select(self, viewport, rect, crossing: bool,
                      additive: bool = False, mode: str | None = None) -> None:
        mode = mode or ("add" if additive else "replace")
        self._send(viewport, {"type": "box", "rect": tuple(rect), "crossing": bool(crossing),
                              "mods": self._MODE_MODS[mode]})
        viewport.update()

    def on_key(self, viewport, key, modifiers) -> bool:
        from PySide6.QtCore import Qt
        name = {Qt.Key_Delete: "Delete", Qt.Key_Backspace: "Backspace"}.get(key)
        if name is None:
            return False
        return self._send(viewport, {"type": "key", "key": name, "mods": _mods(modifiers)})["consumed"]
