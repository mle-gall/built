# SPDX-License-Identifier: GPL-3.0-or-later
"""ToolMachine event conversion, viewport queries and protocol dispatch."""
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

    def __init__(self, viewport, handles=None, tool=None):
        self.vp = viewport
        self.handles = handles
        self.tool = tool

    def call(self, verb, args):
        """A verb the host still owns for a tool: ``tool.host_<verb>``."""
        return getattr(self.tool, "host_" + verb)(self.vp, args)

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
        if name == "place_group":
            answer = run(self.vp.history, name, **args, _pending=self.tool._group,
                         attachment=self.tool._attachment)
            return answer["ok"], self.revision(), 0, answer.get("error")
        if name == "paste_clipboard":
            answer = run(self.vp.history, name, **args, clipboard=self.tool._clip)
            return answer["ok"], self.revision(), 0, answer.get("error")
        if name in ("add_dimension", "add_geo_path", "move_geo_path_node", "add_image_plane",
                    "place_section_plane", "set_north", "solid_operation", "erase_stroke", "follow_me"):
            # Document records and keyed selections use the same coordinates as the viewport.
            # Only mesh drawing commands below need the open store's frame origin.
            try:
                answer = run(self.vp.history, name, **args)
            except (KeyError, IndexError) as error:
                return False, self.revision(), 0, str(error)
            return answer["ok"], self.revision(), 0, answer.get("error")
        if name == "bevel_selection":
            args["_plan"] = self.tool._plan
        if name == "paint_material":
            # Painting a closed, lazy group needs no topology or face count.
            try:
                answer = run(self.vp.history, name, **args,
                             _picked_placement=getattr(self.tool, "_container", None))
            except KeyError as error:
                # Undo can invalidate a cached viewport pick between events.
                return False, self.revision(), 0, str(error)
            return answer["ok"], self.revision(), 0, answer.get("error")
        if name in ("add_guide", "set_group_axes"):
            for key in ("point", "origin", "red", "green"):
                if args.get(key) is not None:
                    args[key] = self._model(args[key])
        for key in ("a", "b", "eye", "anchor", "center"):
            if args.get(key) is not None:
                args[key] = self._model(args[key])
        for key in ("outer", "points"):
            if key in args:
                args[key] = [self._model(p) for p in args[key]]
        if args.get("trim"):
            args["trim"] = [[self._model(a), self._model(b)] for a, b in args["trim"]]
        history = self.vp.history
        before = self.revision()
        faces = len(self.vp.scene.faces)
        try:
            answer = run(history, name, **args)
        except (KeyError, ValueError) as error:
            if name not in ("scale_selection", "rotate_selection", "offset_outline"):
                raise
            # A stale target during hot retype must leave the machine
            # free to redo the original command.
            return False, before, 0, str(error)
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
        from tools import select_host as select
        return self._id(entity), select.pick_kind(entity)

    def query(self, what, x, y):
        from tools import select_host as select
        fn = select.QUERIES.get(what)
        if fn is None:
            return []
        return [(self._id(e), select.pick_kind(e), float(d)) for e, d in fn(self.vp, x, y)]

    def has(self, what):
        from tools import select_host as select
        return select.has(self.vp, what)

    def edge_clear(self, ident, x, y):
        from tools import select_host as select
        return select.edge_clear(self.vp, self.handles.get(ident), x, y)

    def image_in_front(self, ident, x, y):
        from tools import select_host as select
        return self._pick(select.image_in_front(self.vp, self.handles.get(ident), x, y))

    def extension_at(self, x, y):
        pick_item = getattr(self.vp, "pick_extension_item", None)
        hit = pick_item(x, y) if pick_item is not None else None
        return None if hit is None else self._id(hit)

    def cycle_status(self, index, count, ident):
        from tools import select_host as select
        select.cycle_status(self.vp, index, count, self.handles.get(ident))

    def select(self, ids, mode, grow):
        from tools import select_host as select
        select.apply(self.vp, [self.handles.get(i) for i in ids], mode, grow)

    def clear_selection(self):
        self.vp.scene.clear_selection()

    def selection_signature(self):
        return hash(frozenset(self.vp.scene.selection)) & _MASK

    def selection_empty(self):
        return not self.vp.scene.selection

    def box_select(self, rect, crossing, mode):
        from tools import select_host as select
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
        from tools import select_host as select
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
        return [(Vec3(*a), Vec3(*b)) for a, b, _style in self._machine.preview()["lines"]]

    def preview(self):
        return self._machine.preview()


def _point_property(key):
    def get(self):
        return self._point(key)

    def put(self, value):
        self._machine.set(key, _tuple(value))
    return property(get, put)
