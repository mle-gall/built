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
        if name == "add_guide":
            for key in ("point", "origin"):
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
        from tools import select
        return self._id(entity), select.pick_kind(entity)

    def query(self, what, x, y):
        from tools import select
        fn = select.QUERIES.get(what)
        if fn is None:
            return []
        return [(self._id(e), select.pick_kind(e), float(d)) for e, d in fn(self.vp, x, y)]

    def has(self, what):
        from tools import select
        return select.has(self.vp, what)

    def edge_clear(self, ident, x, y):
        from tools import select
        return select.edge_clear(self.vp, self.handles.get(ident), x, y)

    def image_in_front(self, ident, x, y):
        from tools import select
        return self._pick(select.image_in_front(self.vp, self.handles.get(ident), x, y))

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

    def pick_at(self, viewport, x, y, lines=True):
        """What a click at ``(x, y)`` takes (the machine's cascade)."""
        from ingetrazo_core import select_pick
        hit = select_pick(_Host(viewport, self._handles), float(x), float(y), bool(lines))
        return None if hit is None else self._handles.get(hit)

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


class _MachinePlaneLock:
    """``tools.base.PlaneLock``'s state on a planar machine: the machine
    owns it (``Tool::plane_lock``), the mixin's glue reads it here."""

    def __init__(self, machine):
        self._m = machine

    def __getattr__(self, name):
        if name in ("plane_lock", "plane_ref", "hover_plane"):
            return self._m.lock_get(name)
        raise AttributeError(name)

    def __setattr__(self, name, value):
        if name in ("plane_lock", "plane_ref", "hover_plane"):
            self._m.lock_set(name, value)
        else:
            object.__setattr__(self, name, value)

    def toggle(self, axis):
        self._m.lock_toggle(axis)

    def clear(self):
        self._m.lock_clear()

    def locked(self, point, axes=None):
        return self._m.lock_locked(point, axes)

    def own_plane(self, work_plane, axes=None):
        return self._m.lock_own_plane(work_plane, axes)

    def snapped(self, plane, points, names_point, axes=None):
        return self._m.lock_snapped(plane, points, names_point, axes)


def _planar_tool():
    from tools.base import PlaneLock
    return PlaneLock


class RectangleTool(_planar_tool(), RustTool):
    """Rectangle (``rust/ingetrazo_core/src/tools/rectangle.rs``). The
    plane lock is the machine's; ``PlaneLock`` keeps the viewport glue
    (status, redraws, what is under the cursor, the cursor preview)."""

    kind = "rectangle"
    # The second click is the opposite corner: the span to the cursor is
    # the diagonal, never a line to infer along (#389).
    diagonal_span = True
    # "200,100" is two values here (the list comma, #152).
    vcb_comma_lists = True
    wireframe_color = None

    _KEYS = None

    def __init__(self):
        RustTool.__init__(self)
        self._viewport = None

    @property
    def _plane_state(self):
        state = self.__dict__.get("_plane_proxy")
        if state is None:
            state = self.__dict__["_plane_proxy"] = _MachinePlaneLock(self._machine)
        return state

    start_point = _point_property("start_point")
    hover_point = _point_property("hover_point")

    @property
    def chain_first_point(self):
        return None             # no close-polygon snap onto the first corner

    @chain_first_point.setter
    def chain_first_point(self, _value):
        pass

    @property
    def work_plane(self):
        plane = self._state("work_plane")
        return None if plane is None else (Vec3(*plane[0]), Vec3(*plane[1]))

    @work_plane.setter
    def work_plane(self, plane):
        self._machine.set("work_plane", None if plane is None else (_tuple(plane[0]), _tuple(plane[1])))

    @property
    def _from_center(self):
        return bool(self._state("from_center"))

    @_from_center.setter
    def _from_center(self, on):
        self._machine.set("from_center", bool(on))

    @property
    def icon(self):
        return "rectangle_center" if self._from_center else "rectangle"

    @icon.setter
    def icon(self, _value):
        pass

    @property
    def _last(self):
        return True if self._state("retype") else None

    def _set_from_center(self, viewport, on):
        self._from_center = on
        apply = getattr(viewport, "_apply_tool_cursor", None)
        if callable(apply):
            apply()

    def plane_points(self):
        if self.start_point is None:
            return []
        return [self.start_point, self.hover_point]

    def _frame(self, viewport):
        from core import axes
        from core.snap import POINT_SNAP_KINDS
        kind = getattr(self, "snap_kind", None)
        self._send(viewport, {"type": "frame", "axes": axes.frame(),
                              "names_point": None if kind is None else kind in POINT_SNAP_KINDS})

    def _send_framed(self, viewport, event):
        self._frame(viewport)
        return self._send(viewport, event)

    def on_activate(self, viewport) -> None:
        self._send_framed(viewport, {"type": "activate"})
        self.wireframe_color = None
        self._set_from_center(viewport, False)    # a pick-up starts corner-wise

    def on_deactivate(self, viewport) -> None:
        self._send_framed(viewport, {"type": "deactivate"})
        self.wireframe_color = None

    def on_cancel(self, viewport) -> None:
        self._send_framed(viewport, {"type": "cancel"})
        self.wireframe_color = None
        viewport.update()

    def on_click(self, ctx) -> None:
        self.note_plane(ctx.viewport)
        reply = self._send_framed(ctx.viewport, self._pointer("click", ctx))
        if self.start_point is None:
            self.wireframe_color = None
        ctx.viewport.update()
        return reply

    def on_hover(self, ctx) -> None:
        self.note_plane(ctx.viewport)
        self._viewport = ctx.viewport
        self._send_framed(ctx.viewport, self._pointer("hover", ctx))
        self.wireframe_color = self.lock_color()
        ctx.viewport.update()

    def on_value(self, viewport, value) -> bool:
        consumed = self._send_framed(viewport, {"type": "value", "value": value})["consumed"]
        if consumed and self.start_point is None:
            self.wireframe_color = None
        return consumed

    def on_key(self, viewport, key, modifiers) -> bool:
        from PySide6.QtCore import Qt
        name = {Qt.Key_Control: "Control", Qt.Key_Down: "ArrowDown", Qt.Key_Right: "ArrowRight",
                Qt.Key_Left: "ArrowLeft", Qt.Key_Up: "ArrowUp"}.get(key)
        if name is not None and self._send_framed(
                viewport, {"type": "key", "key": name, "mods": _mods(modifiers)})["consumed"]:
            if name == "Control":
                self._set_from_center(viewport, self._from_center)
                self.refresh_hint(viewport)
            update = getattr(viewport, "update", None)
            if callable(update):
                update()
            return True
        return self.plane_lock_key(viewport, key)

    def on_key_release(self, viewport, key) -> bool:
        from PySide6.QtCore import Qt
        if key != Qt.Key_Control:
            return False
        tapped = getattr(viewport, "ctrl_tapped", None)
        reply = self._send_framed(viewport, {"type": "key_release", "key": "Control",
                                             "tapped": not callable(tapped) or bool(tapped())})
        if reply["consumed"]:
            self._set_from_center(viewport, self._from_center)
        return reply["consumed"]

    def rubber_band_lines(self):
        if self.hover_point is None:
            return []
        if self.start_point is None:
            return self._cursor_preview()
        return RustTool.rubber_band_lines(self)

    def value_label(self):
        """``(text, point)``: the floating ``width × height`` readout, with
        "(Cuadrado)" when the sides are equal."""
        label = self._state("label")
        if label is None:
            return None
        from core.units import fmt_pair
        du, dv, square, mid = label
        text = fmt_pair(du, dv) + ("  (Cuadrado)" if square else "")
        return text, Vec3(*mid)

    def _cursor_preview(self):
        """The little square on the cursor before the first corner, on the
        plane the rectangle would take (view glue: it needs the pixel scale)."""
        vp = self._viewport
        if vp is None:
            return []
        from core.axes import plane_axes
        point, normal = self.preview_plane(vp, self.hover_point)
        u, v = plane_axes(normal)
        scale = self.plane_preview_scale(vp, self.hover_point, u, v)
        if scale is None:
            return []
        h = 0.5 * self.PREVIEW_PX * scale
        c = self.hover_point
        if self._from_center:
            pts = [c + u * h + v * h, c - u * h + v * h, c - u * h - v * h, c + u * h - v * h]
        else:
            pts = [c, c + u * (2 * h), c + (u + v) * (2 * h), c + v * (2 * h)]
        return [(pts[i], pts[(i + 1) % 4]) for i in range(4)]

    def _flag(name):
        def get(self):
            return self._state(name)

        def put(self, value):
            self._machine.set(name, value)
        return property(get, put)

    _live_side = _flag("live_side")
    _arrow_square_lock = _flag("arrow_square")
    _shift_square_lock = _flag("shift_square")

    def _reset(self):
        self._machine.set("reset", True)
        self.wireframe_color = None

    def _query(self, what, arg):
        self._frame(self._viewport)
        return self._machine.query(what, arg)

    def _span(self, cursor):
        a, b = self._query("span", _tuple(cursor))
        return Vec3(*a), Vec3(*b)

    def _dimensions(self, a, b):
        return tuple(self._query("dimensions", (_tuple(a), _tuple(b))))

    def _square_locked(self):
        return bool(self._state("square"))

    def _square_corner(self, a, b):
        far, square = self._query("square_corner", (_tuple(a), _tuple(b)))
        return Vec3(*far), bool(square)

    def _corners(self, a, b):
        """The four corners of the rectangle spanning ``a``–``b`` on the
        drawing plane."""
        return [Vec3(*p) for p in self._query("corners", (_tuple(a), _tuple(b)))]

    def _degenerate_reason(self, world) -> str:
        from core.i18n import tr
        template, args = self._query("degenerate", _tuple(world))
        return tr(template, **args) if args else tr(template)


def _radial_bases():
    from tools.base import AxisMagnet, PlaneLock
    return AxisMagnet, PlaneLock


class _RadialTool(*_radial_bases(), RustTool):
    """Circle and Polygon (``rust/ingetrazo_core/src/tools/circle.rs``): a
    centre, then a radius. The machine owns the plane lock, the side count,
    the guide mode and the retype window; this keeps the viewport glue (the
    status, the axis colour, the cursor's ring, the readout)."""

    vcb_label = "Radius"
    #: Whether Ctrl toggles the guide-circle mode (only the Circle tool).
    _guide_capable = False
    wireframe_color = None

    def __init__(self):
        RustTool.__init__(self)
        self._viewport = None

    @property
    def _plane_state(self):
        state = self.__dict__.get("_plane_proxy")
        if state is None:
            state = self.__dict__["_plane_proxy"] = _MachinePlaneLock(self._machine)
        return state

    start_point = _point_property("start_point")
    hover_point = _point_property("hover_point")

    @property
    def work_plane(self):
        plane = self._state("work_plane")
        return None if plane is None else (Vec3(*plane[0]), Vec3(*plane[1]))

    @work_plane.setter
    def work_plane(self, plane):
        self._machine.set("work_plane", None if plane is None else (_tuple(plane[0]), _tuple(plane[1])))

    @property
    def sides(self):
        return int(self._state("sides"))

    @sides.setter
    def sides(self, n):
        self._machine.set("sides", int(n))

    @property
    def _guide_mode(self):
        return bool(self._state("guide_mode"))

    @_guide_mode.setter
    def _guide_mode(self, on):
        self._machine.set("guide_mode", bool(on))

    @property
    def _last(self):
        """The retype token of the shape just drawn, or None."""
        return self._state("last_token")

    def _frame(self, viewport):
        from core import axes
        from core.snap import POINT_SNAP_KINDS
        kind = getattr(self, "snap_kind", None)
        self._send(viewport, {"type": "frame", "axes": axes.frame(),
                              "names_point": None if kind is None else kind in POINT_SNAP_KINDS})

    def _send_framed(self, viewport, event):
        self._frame(viewport)
        return self._send(viewport, event)

    # ---- the value box ------------------------------------------------------
    def vcb_caption(self) -> str:
        """'Sides' before the centre, 'Radius' after — and right after a
        shape is drawn, while a typed number still resizes it (#414)."""
        if self.start_point is not None or self._radius_retype_open():
            return "Radius"
        return "Sides"

    def _radius_retype_open(self) -> bool:
        token = self._last
        return (self.start_point is None and token is not None and self._viewport is not None
                and _Host(self._viewport).revision() == token)

    def value_is_unitless(self) -> bool:
        """A side count before the centre (#176); a radius after it, and
        right after a shape (#414)."""
        return self.start_point is None and not self._radius_retype_open()

    # ---- events -------------------------------------------------------------
    def on_activate(self, viewport) -> None:
        self._send_framed(viewport, {"type": "activate"})
        self.wireframe_color = None

    def on_deactivate(self, viewport) -> None:
        self._send_framed(viewport, {"type": "deactivate"})
        self.wireframe_color = None

    def on_cancel(self, viewport) -> None:
        self._send_framed(viewport, {"type": "cancel"})
        self.wireframe_color = None
        viewport.update()

    def on_click(self, ctx) -> None:
        self.note_plane(ctx.viewport)
        self._viewport = ctx.viewport
        self._send_framed(ctx.viewport, self._pointer("click", ctx))
        if self.start_point is None:
            self.wireframe_color = None
        ctx.viewport.update()

    def on_hover(self, ctx) -> None:
        self.note_plane(ctx.viewport)
        self._viewport = ctx.viewport
        self._send_framed(ctx.viewport, self._pointer("hover", ctx))
        snap = ctx.snap
        if (snap is not None and snap.kind == "axis"
                and self.plane_lock is None and self.plane_ref is None):
            # A radius magnetised onto an axis rings in the AXIS colour,
            # not the plane's (#962); a plane lock keeps its own.
            self.wireframe_color = None
        else:
            self.wireframe_color = self.lock_color()
        ctx.viewport.update()

    def on_segments_value(self, viewport, n: int) -> bool:
        """«24s»: the side count, typed at any moment; right after a shape,
        that shape is rebuilt with it, one undo step (#305)."""
        self._viewport = viewport
        consumed = self._send_framed(viewport, {"type": "segments", "count": int(n)})["consumed"]
        if consumed:
            viewport.update()
        return consumed

    def on_value(self, viewport, value) -> bool:
        self._viewport = viewport
        if isinstance(value, tuple):
            return False
        consumed = self._send_framed(viewport, {"type": "value", "value": float(value)})["consumed"]
        if consumed:
            if self.start_point is None:
                self.wireframe_color = None
            viewport.update()
        return consumed

    def _refresh(self, viewport):
        hint = getattr(viewport, "refresh_status_hint", None)
        if callable(hint):
            hint()
        viewport.update()

    def on_key(self, viewport, key, modifiers) -> bool:
        from PySide6.QtCore import Qt
        if key == Qt.Key_Control and self._guide_capable:
            self._send_framed(viewport, {"type": "key", "key": "Control", "mods": _mods(modifiers)})
            self._refresh(viewport)
            return True
        return self.plane_lock_key(viewport, key)

    def on_key_release(self, viewport, key) -> bool:
        from PySide6.QtCore import Qt
        if key != Qt.Key_Control:
            return False
        tapped = getattr(viewport, "ctrl_tapped", None)
        reply = self._send_framed(viewport, {"type": "key_release", "key": "Control",
                                             "tapped": not callable(tapped) or bool(tapped())})
        if reply["consumed"]:
            self._refresh(viewport)
        return reply["consumed"]

    def status_clause(self) -> str:
        if not self._guide_capable:
            return ""
        from core.i18n import tr
        return tr("Ctrl = guide")   # short: it rides the line of a long hint

    # ---- preview ------------------------------------------------------------
    def rubber_band_lines(self):
        if self.hover_point is None:
            return []
        if self.start_point is None:
            return self._cursor_preview()
        return RustTool.rubber_band_lines(self)

    def guide_preview_lines(self):
        """The dashed rim of the guide circle the next click leaves."""
        return [(Vec3(*a), Vec3(*b)) for a, b in self._state("guide_ring") or ()]

    def value_label(self):
        """The radius DRAWN, the rim projected onto the plane, not the
        straight distance to the cursor."""
        r = self._state("radius")
        if r is None:
            return None
        from core.i18n import tr
        from core.units import fmt_len
        return ("R " + fmt_len(r) + "  (" + tr("{n} sides", n=self.sides) + ")",
                self.hover_point)

    def _cursor_preview(self):
        """The ring on the cursor before the centre, at a fixed screen size
        on the plane it would take (view glue: it needs the pixel scale)."""
        import math
        vp = self._viewport
        if vp is None:
            return []
        from core.axes import plane_axes
        point, normal = self.preview_plane(vp, self.hover_point)
        u, v = plane_axes(normal)
        scale = self.plane_preview_scale(vp, self.hover_point, u, v)
        if scale is None:
            return []
        r = self.PREVIEW_PX * scale
        n = max(int(self.sides), 3)
        pts = [self.hover_point + u * (r * math.cos(2 * math.pi * i / n))
               + v * (r * math.sin(2 * math.pi * i / n)) for i in range(n)]
        return [(pts[i], pts[(i + 1) % n]) for i in range(n)]

    # ---- what the snap engine and the tests read ------------------------------
    def plane_points(self):
        if self.start_point is None:
            return []
        return [self.start_point, self.hover_point]

    def _query(self, what, arg):
        if self._viewport is not None:
            self._frame(self._viewport)
        return self._machine.query(what, arg)

    def _axes(self):
        u, v = self._query("axes", None)
        return Vec3(*u), Vec3(*v)

    def _points(self, center, rim):
        return [Vec3(*p) for p in self._query("points", (_tuple(center), _tuple(rim)))]

    def _commit(self, viewport, pts, axes=None, center=None) -> None:
        """A closed curve with its face through the protocol (``add_face``
        with ``curve``), outside the machine's retype window."""
        _Host(viewport).run("add_face", json.dumps(
            {"outer": [list(_tuple(p)) for p in pts], "curve": True}))
        viewport.update()

    def _reset(self):
        self._machine.set("reset", True)
        self.wireframe_color = None


class CircleTool(_RadialTool):
    kind = "circle"
    _guide_capable = True


class PolygonTool(_RadialTool):
    kind = "polygon"
    shortcut = None  # Polygon has no default; G = Make Component


class RotatedRectangleTool(*_radial_bases(), RustTool):
    """Rotated Rectangle (``rust/ingetrazo_core/src/tools/rotated_rectangle.rs``).
    The machine owns the three clicks, the protractors' rules and the plane
    lock; this sends what only the view knows (the face under the first
    click, the cursor's ray, the axis lock, the protractor's screen size)."""

    kind = "rotated_rectangle"
    shortcut = None
    # The arrows choose the base edge's plane before the first click; once
    # the base is down they are the viewport's axis lock, which the width
    # reads (``ViewReading.axis_lock``).
    plane_lock_after_the_first_point = False
    wireframe_color = None

    def __init__(self):
        RustTool.__init__(self)
        self._viewport = None

    @property
    def _plane_state(self):
        state = self.__dict__.get("_plane_proxy")
        if state is None:
            state = self.__dict__["_plane_proxy"] = _MachinePlaneLock(self._machine)
        return state

    start_point = _point_property("start_point")
    base_point = _point_property("base_point")
    hover_point = _point_property("hover_point")

    @property
    def work_plane(self):
        plane = self._state("work_plane")
        return None if plane is None else (Vec3(*plane[0]), Vec3(*plane[1]))

    @work_plane.setter
    def work_plane(self, plane):
        self._machine.set("work_plane", None if plane is None else (_tuple(plane[0]), _tuple(plane[1])))

    @property
    def angle(self):
        return float(self._state("angle"))

    @angle.setter
    def angle(self, value):
        self._machine.set("angle", float(value))

    @property
    def vcb_label(self) -> str:
        return "Length; angle" if self.base_point is None else "Width; angle"

    def magnet_on(self) -> bool:
        # The base edge is a direction from the first corner; the width is
        # measured square off that edge.
        return self.base_point is None

    @property
    def diagonal_span(self) -> bool:
        # For the width, the span from the first corner is a diagonal (#389).
        return self.base_point is not None

    # ---- what only the view knows -----------------------------------------
    def _view(self, viewport, ctx=None):
        """``ViewReading`` for the event about to go: the cursor's ray, the
        axis lock, the next protractor's size and nearness, the view's
        drawing plane."""
        import math
        from core.axes import AXES, plane_axes
        lock = getattr(viewport, "axis_lock", None)
        reading = {"type": "view",
                   "axis_lock": _tuple(AXES[lock]) if lock and lock in AXES else None}
        normal = getattr(viewport, "_work_plane_normal", None)
        if self.work_plane is None and callable(normal):
            reading["plane_normal"] = _tuple(normal())
        if ctx is not None:
            screen = ctx.screen
            ray = getattr(viewport, "_pixel_to_ray", None)
            kind = getattr(getattr(ctx, "snap", None), "kind", None)
            if (ray is not None and self.base_point is not None
                    and not self._machine.query("point_snap", kind)):
                o, d = ray(screen.x(), screen.y())
                if o is not None and d is not None:
                    reading["ray"] = (_tuple(o), _tuple(d))
            disc = self._state("disc")
            w2p = getattr(viewport, "_world_to_pixel", None)
            if disc is not None:
                radius, near = None, False
                centre = Vec3(*disc[0])
                u, _v = plane_axes(Vec3(*disc[1]))
                p0 = p1 = None
                if w2p is not None:
                    p0, p1 = w2p(centre), w2p(centre + u)
                if p0 is not None and p1 is not None:
                    from tools.protractor import DISC_PX
                    px = math.hypot(p1[0] - p0[0], p1[1] - p0[1])
                    if px > 1e-6:
                        radius = DISC_PX / px
                    near = math.hypot(screen.x() - p0[0], screen.y() - p0[1]) <= DISC_PX * 1.25
                reading["protractor"] = (radius, near)
        self._send(viewport, reading)

    def _face_plane(self, ctx):
        """The face under the first click, as the plane to draw on."""
        pick = (getattr(ctx.viewport, "pick_face_placement", None)
                or getattr(ctx.viewport, "pick_face_any", None))
        if pick is None:
            return None
        face, group = pick(ctx.screen.x(), ctx.screen.y())
        if face is None:
            return None
        from core.snap import face_plane_world
        _point, normal = face_plane_world(face, getattr(group, "xform", None))
        if normal is None or normal.lengthSquared() < 0.5:
            return None
        return _tuple(ctx.world), _tuple(normal)

    def _framed(self, viewport, event, ctx=None):
        from core import axes
        self._send(viewport, {"type": "frame", "axes": axes.frame()})
        self._view(viewport, ctx)
        return self._send(viewport, event)

    # ---- events -------------------------------------------------------------
    def on_activate(self, viewport) -> None:
        self._framed(viewport, {"type": "activate"})

    def on_deactivate(self, viewport) -> None:
        self._framed(viewport, {"type": "deactivate"})

    def on_cancel(self, viewport) -> None:
        self._framed(viewport, {"type": "cancel"})
        viewport.update()

    def on_click(self, ctx) -> None:
        self._viewport = ctx.viewport
        if self.start_point is None:
            self._send(ctx.viewport, {"type": "work_plane", "plane": self._face_plane(ctx)})
        self._framed(ctx.viewport, self._pointer("click", ctx), ctx)
        ctx.viewport.update()

    def on_hover(self, ctx) -> None:
        self._viewport = ctx.viewport
        self._framed(ctx.viewport, self._pointer("hover", ctx), ctx)
        ctx.viewport.update()

    def on_value(self, viewport, value) -> bool:
        """``3`` = 3 m keeping the angle shown; ``3;90`` = 3 m standing
        square to the plane. The angle field reads without the document's
        unit (``_degrees``)."""
        if isinstance(value, tuple) and len(value) == 2:
            value = (float(value[0]), _degrees(value[1]))
        reply = self._framed(viewport, {"type": "value", "value": value})
        if reply["consumed"]:
            viewport.update()
        return reply["consumed"]

    def on_key(self, viewport, key, modifiers) -> bool:
        return self.plane_lock_key(viewport, key)

    # ---- preview ------------------------------------------------------------
    def value_label(self):
        label = self._state("label")
        if label is None:
            return None
        from core.units import fmt_len, fmt_pair
        if label[0] == "edge":
            _, length, angle, mid = label
            text = fmt_len(length) + ("" if angle is None else f"   {angle:.0f}°")
            return text, Vec3(*mid)
        _, length, width, angle, mid = label
        text = fmt_pair(length, width) + (f"   {angle:.0f}°" if abs(angle) > 0.05 else "")
        return text, Vec3(*mid)

    @property
    def _locked(self):
        """The width's direction the axis lock (or Shift) imposes, as of
        the last hover."""
        d = self._state("locked")
        return None if d is None else Vec3(*d)

    def locked_dir(self, viewport):
        """The direction the view's axis lock imposes on the width, or None
        (no lock, or the lock is the base edge's own axis)."""
        self._view(viewport)
        d = self._machine.query("locked_dir", None)
        return None if d is None else Vec3(*d)

    def _corners(self, width, angle=None):
        return [Vec3(*p) for p in self._machine.query("corners", (float(width), angle))]

    def _width_and_angle(self, cursor, forced=None):
        w, a = self._machine.query("width_and_angle", (_tuple(cursor), _tuple(forced)))
        return w, a

    def _perp(self):
        return Vec3(*self._machine.query("perp", None))

    def _normal(self):
        return Vec3(*self._machine.query("normal", None))

    def _reset(self):
        self._machine.set("reset", True)


def _degrees(value) -> float:
    """An angle typed in the value box: the parser reads a bare field as a
    LENGTH in the document's unit (90 → 0.09 in millimetres); an angle has
    none."""
    from core.units import bare_number_scale
    scale = bare_number_scale()
    return float(value) / scale if scale else float(value)


# ---- Arcs ---------------------------------------------------------------------

#: Magenta of the arc tangent to BOTH edges of a corner, the fillet.
COLOR_FILLET = (0.85, 0.30, 0.80)


class _ArcBase(*_radial_bases(), RustTool):
    """The arcs (``rust/ingetrazo_core/src/tools/arc.rs``): the machine owns
    the clicks, the plane lock, the segment count and the commit; this keeps
    the viewport glue (status, colours, readouts)."""

    wireframe_color = None

    def __init__(self):
        RustTool.__init__(self)
        self._viewport = None

    @property
    def _plane_state(self):
        state = self.__dict__.get("_plane_proxy")
        if state is None:
            state = self.__dict__["_plane_proxy"] = _MachinePlaneLock(self._machine)
        return state

    start_point = _point_property("start_point")
    hover_point = _point_property("hover_point")

    @property
    def work_plane(self):
        plane = self._state("work_plane")
        return None if plane is None else (Vec3(*plane[0]), Vec3(*plane[1]))

    @work_plane.setter
    def work_plane(self, plane):
        self._machine.set("work_plane", None if plane is None else (_tuple(plane[0]), _tuple(plane[1])))

    @property
    def segments(self):
        return self._state("segments")

    @segments.setter
    def segments(self, n):
        self._machine.set("segments", None if n is None else int(n))

    def _frame(self, viewport):
        from core import axes
        from core.snap import POINT_SNAP_KINDS
        kind = getattr(self, "snap_kind", None)
        self._send(viewport, {"type": "frame", "axes": axes.frame(),
                              "names_point": None if kind is None else kind in POINT_SNAP_KINDS})

    def _send_framed(self, viewport, event):
        self._frame(viewport)
        return self._send(viewport, event)

    def _after(self, viewport):
        if self.start_point is None:
            self.wireframe_color = None
        viewport.update()

    def value_is_unitless(self) -> bool:
        """Before the first click the typed number is a COUNT (#176)."""
        return self.start_point is None

    def on_activate(self, viewport) -> None:
        self._send_framed(viewport, {"type": "activate"})
        self.wireframe_color = None

    def on_deactivate(self, viewport) -> None:
        self._send_framed(viewport, {"type": "deactivate"})
        self.wireframe_color = None

    def on_cancel(self, viewport) -> None:
        self._send_framed(viewport, {"type": "cancel"})
        self._after(viewport)

    def on_click(self, ctx) -> None:
        self.note_plane(ctx.viewport)
        self._viewport = ctx.viewport
        self._send_framed(ctx.viewport, self._pointer("click", ctx))
        self._after(ctx.viewport)

    def on_hover(self, ctx) -> None:
        self.note_plane(ctx.viewport)
        self._viewport = ctx.viewport
        self._send_framed(ctx.viewport, self._pointer("hover", ctx))
        self.wireframe_color = self._cue_colour()
        ctx.viewport.update()

    def _cue_colour(self):
        return self.lock_color()

    def on_value(self, viewport, value) -> bool:
        self._viewport = viewport
        if isinstance(value, tuple):
            return False
        consumed = self._send_framed(viewport, {"type": "value", "value": float(value)})["consumed"]
        self._after(viewport)
        return consumed

    def on_segments_value(self, viewport, n) -> bool:
        self._viewport = viewport
        consumed = self._send_framed(viewport, {"type": "segments", "count": int(n)})["consumed"]
        if consumed:
            viewport.update()
        return consumed

    def on_key(self, viewport, key, modifiers) -> bool:
        return self.plane_lock_key(viewport, key)

    def _query(self, what, arg=None):
        if self._viewport is not None:
            self._frame(self._viewport)
        return self._machine.query(what, arg)

    def _axes(self):
        u, v = self._query("axes")
        return Vec3(*u), Vec3(*v)

    def _reset(self):
        self._machine.set("reset", True)
        self.wireframe_color = None


class _LastFillet(type(RustTool)):
    """``ArcTool.last_fillet_d``: the distance a double-click repeats, one
    for every arc (the machine's global)."""

    @property
    def last_fillet_d(cls):
        from ingetrazo_core import arc_last_fillet_d
        return arc_last_fillet_d()

    @last_fillet_d.setter
    def last_fillet_d(cls, value):
        from ingetrazo_core import arc_last_fillet_d
        arc_last_fillet_d(None if value is None else float(value), True)


def _edge_json(handles, e):
    return {"id": handles.id(e), "a": _tuple(e.a), "b": _tuple(e.b), "faces": bool(e.faces)}


class ArcTool(_ArcBase, metaclass=_LastFillet):
    """2-point arc. The host answers the topology the fillet reads (the
    ``host_*`` queries) and projects points to pixels."""

    kind = "arc"
    vcb_label = "Bulge"

    end_point = _point_property("end_point")

    def magnet_on(self) -> bool:
        # The chord end is a direction from the start; the bulge is not.
        return self.end_point is None

    @property
    def _fillet(self):
        f = self._state("fillet")
        return None if f is None else tuple(Vec3(*p) for p in f)

    @property
    def _equidistant(self):
        e = self._state("equidistant")
        return None if e is None else (Vec3(*e[0]), Vec3(*e[1]), self._handles.get(e[2]))

    @property
    def _start_edge(self):
        e = self._state("start_edge")
        return None if e is None else (self._handles.get(e[0]), Vec3(*e[1]))

    def _tangent_at_start(self, viewport):
        """The direction an existing arc leaves the start (``host_tangent_at``)."""
        if self.start_point is None:
            return None
        t = self.host_tangent_at(viewport, {"point": _tuple(self.start_point)})
        return None if t is None else Vec3(*t)

    @property
    def _bulge_kind(self):
        return self._state("kind")

    @property
    def _snap_bulge(self):
        return self._state("snap_bulge")

    @property
    def _tangent_dir(self):
        return self._point("tangent_dir")

    def _cue_colour(self):
        from core.snap import COLOR_MIDPOINT, COLOR_TANGENT
        cue = self._state("cue")
        colour = {"tangent": COLOR_TANGENT, "fillet": COLOR_FILLET, "half": COLOR_MIDPOINT}.get(cue)
        return (*colour, 1.0) if colour is not None else self.lock_color()

    def vcb_caption(self) -> str:
        if self.start_point is None:
            return "Segments"
        return "Radius" if self._fillet is not None else "Bulge"

    def on_double_click(self, ctx) -> None:
        self.note_plane(ctx.viewport)
        self._viewport = ctx.viewport
        self._send_framed(ctx.viewport, self._pointer("double_click", ctx))
        self._after(ctx.viewport)

    def on_radius_value(self, viewport, radius) -> bool:
        """«2r»: the radius instead of the bulge."""
        self._viewport = viewport
        consumed = self._send_framed(viewport, {"type": "radius", "value": float(radius)})["consumed"]
        self._after(viewport)
        return consumed

    def rubber_band_lines(self):
        return RustTool.rubber_band_lines(self)

    def value_label(self):
        label = self._state("label")
        if label is None:
            return None
        from core.i18n import tr
        from core.units import fmt_len
        kind, value, mid = label
        text = {"tangent_edge": lambda: tr("Tangent to edge"),
                "half": lambda: tr("Half circle") + "  " + fmt_len(value),
                "fillet": lambda: tr("Tangent to edge") + "  " + fmt_len(value),
                "tangent": lambda: tr("Tangent at vertex") + "  " + fmt_len(value),
                "bulge": lambda: "Bulge " + fmt_len(value)}[kind]()
        return text, Vec3(*mid)

    def plane_points(self):
        if self.start_point is None:
            return []
        return [self.start_point, self.end_point if self.end_point is not None else self.hover_point]

    def _points(self, cursor, bulge=None):
        return [Vec3(*p) for p in self._query("points", (_tuple(cursor), bulge))]

    def _bulge_for(self, cursor):
        return self._query("bulge_for", (_tuple(cursor),))

    def _tangent_bulge(self):
        return self._query("tangent_bulge")

    # ---- what the host answers: topology and pixels -------------------------
    def _mesh(self, viewport):
        return getattr(getattr(viewport, "scene", None), "mesh", None)

    def host_edge_under(self, viewport, args):
        """The edge whose interior holds the point: the hovered one first."""
        mesh = self._mesh(viewport)
        if mesh is None:
            return None
        from core.topology import _point_on_seg_incl
        P = Vec3(*args["point"])

        def interior(e) -> bool:
            if not _point_on_seg_incl(P, e.a, e.b, 1e-4):
                return False
            return (P - e.a).length() > 1e-4 and (P - e.b).length() > 1e-4 and e.length() > 1e-6
        hovered = getattr(viewport, "_hover_edge", None)
        v0 = getattr(hovered, "v0", None)
        if v0 is not None and hovered in getattr(v0, "edges", ()) and interior(hovered):
            return _edge_json(self._handles, hovered)
        for e in mesh.edges:
            if interior(e):
                return _edge_json(self._handles, e)
        return None

    def host_corner_edges(self, viewport, args):
        """The edges leaving each end of the start edge."""
        edge = self._handles.get(args["edge"])
        rows = []
        for V in (edge.v0, edge.v1):
            for B in V.edges:
                if B == edge:
                    continue
                rows.append({"v": _tuple(V.position), "b": _edge_json(self._handles, B),
                             "far": _tuple(B.other(V).position)})
        return rows

    def host_rounded_corners(self, viewport, args):
        """For each end of the edge a curve leaves, the straight edges at the
        curve's other end (#1354); the machine finds where the lines meet."""
        mesh = self._mesh(viewport)
        edge = self._handles.get(args["edge"])
        rows = []
        if mesh is None or edge is None:
            return rows
        for V in (edge.v0, edge.v1):
            for C in V.edges:
                if C is edge or getattr(C, "curve", None) is None:
                    continue
                chain = mesh.curve_edges(C)
                seen = {}
                for c in chain:
                    for vv in (c.v0, c.v1):
                        seen[id(vv)] = (vv, seen.get(id(vv), (vv, 0))[1] + 1)
                for V2, count in seen.values():
                    if count != 1 or V2 is V:
                        continue
                    for B in V2.edges:
                        if (B in chain or getattr(B, "curve", None) is not None
                                or getattr(B, "soft", False)):
                            continue
                        rows.append({"v": _tuple(V.position), "edge_far": _tuple(edge.other(V).position),
                                     "v2": _tuple(V2.position), "far": _tuple(B.other(V2).position),
                                     "b": _edge_json(self._handles, B)})
        return rows

    def host_two_edge_corners(self, viewport, args):
        """Corners where exactly two edges meet, one on a face, both at least
        ``min_length`` long (a double-click repeats the last fillet there)."""
        mesh = self._mesh(viewport)
        rows = []
        if mesh is None:
            return rows
        d = float(args["min_length"])
        for V in mesh.vertices:
            edges = list(V.edges)
            if len(edges) != 2:
                continue
            A, B = edges
            if not (A.faces or B.faces) or A.length() < d or B.length() < d:
                continue
            rows.append({"v": _tuple(V.position), "a": _edge_json(self._handles, A),
                         "b": _edge_json(self._handles, B),
                         "a_far": _tuple(A.other(V).position), "b_far": _tuple(B.other(V).position)})
        return rows

    def host_vertex_degree(self, viewport, args):
        mesh = self._mesh(viewport)
        vert = mesh.vertex_at(Vec3(*args["point"])) if mesh is not None else None
        return None if vert is None else len(vert.edges)

    def host_to_px(self, viewport, args):
        project = getattr(viewport, "_world_to_pixel", None)
        if not callable(project):
            return None
        out = []
        for p in args["points"]:
            q = project(Vec3(*p))
            out.append(None if q is None else [float(q[0]), float(q[1])])
        return out

    def host_tangent_at(self, viewport, args):
        """The direction an existing arc leaves its END vertex at the point:
        the view's resident curves, else the loose mesh and every placement."""
        mesh = self._mesh(viewport)
        P = Vec3(*args["point"])
        if mesh is None:
            return None
        resident = getattr(viewport, "reference_tangent", None)
        if callable(resident):
            return _tuple(resident(P))
        spaces = [(mesh, None)]
        placements = getattr(viewport, "_placements", None)
        if callable(placements):
            for g in placements():
                gm = getattr(g, "mesh", None)
                if gm is not None and gm.edges and not getattr(g, "billboard", False):
                    spaces.append((gm, getattr(g, "xform", None)))
        for gm, xf in spaces:
            inv = None
            if xf is not None:
                inv, ok = xf.inverted()
                if not ok:
                    continue
            t = _tangent_in_mesh(gm, inv.map(P) if inv is not None else P)
            if t is None:
                continue
            if xf is not None:
                t = xf.mapVector(t)
                if t.length() < 1e-9:
                    continue
                t = t.normalized()
            return _tuple(t)
        return None


def _tangent_in_mesh(mesh, P):
    """The tangent at ``P`` of a curve in ``mesh`` that ends there: the
    circle through the curve's points, square to its radius."""
    from core.snap import fit_circle
    for edge in mesh.edges:
        if getattr(edge, "curve", None) is None:
            continue
        if (edge.a - P).length() > 1e-6 and (edge.b - P).length() > 1e-6:
            continue
        chain = mesh.curve_edges(edge)
        incident = [e for e in chain if (e.a - P).length() <= 1e-6 or (e.b - P).length() <= 1e-6]
        if len(incident) != 1:
            continue                       # not the arc's end
        e = incident[0]
        Q = e.b if (e.a - P).length() <= 1e-6 else e.a
        pts = {}
        for c in chain:
            for vv in (c.v0, c.v1):
                pts[vv.key] = vv.position
        fit = fit_circle(list(pts.values())) if len(pts) >= 3 else None
        if fit is None:
            continue
        radial = P - fit[0]
        if radial.length() < 1e-9:
            continue
        r_hat = radial.normalized()
        leaving = P - Q
        t = leaving - r_hat * Vec3.dotProduct(leaving, r_hat)
        if t.length() < 1e-9:
            continue
        return t.normalized()
    return None


class ThreePointArcTool(_ArcBase):
    """3-point arc: the circle through the three clicks."""

    kind = "arc3"
    vcb_label = "Segments"

    mid_point = _point_property("mid_point")

    def on_value(self, viewport, value) -> bool:
        if isinstance(value, tuple) or self.start_point is not None:
            return False
        return _ArcBase.on_value(self, viewport, value)

    def plane_points(self):
        if self.start_point is None:
            return []
        if self.mid_point is None:
            return [self.start_point, self.hover_point]
        return [self.start_point, self.mid_point, self.hover_point]

    def _points(self, end):
        return [Vec3(*p) for p in self._query("points", (_tuple(end),))]


class CenterArcTool(_ArcBase):
    """Centre arc: centre, the 0° arm, the sweep."""

    kind = "center_arc"
    vcb_label = "Angle"

    arm_point = _point_property("arm_point")

    def value_is_unitless(self) -> bool:
        """A segment count and a sweep angle are not lengths (#176)."""
        return self.start_point is None or self.arm_point is not None

    def vcb_caption(self) -> str:
        if self.start_point is None:
            return "Segments"
        return "Angle" if self.arm_point is not None else "Radius"

    def value_label(self):
        label = self._state("label")
        if label is None:
            return None
        kind, value, at = label
        if kind == "radius":
            from core.units import fmt_len
            return "R " + fmt_len(value), Vec3(*at)
        from core.units import fmt_angle_deg
        return fmt_angle_deg(value, signed=True), Vec3(*at)

    def plane_points(self):
        if self.start_point is None:
            return []
        return [self.start_point, self.arm_point if self.arm_point is not None else self.hover_point]

    def _sweep_to(self, cursor):
        return self._query("sweep_to", (_tuple(cursor),))

    def _points(self, sweep_deg):
        return [Vec3(*p) for p in self._query("points", (float(sweep_deg),))]


class PieTool(CenterArcTool):
    """Pie: the centre arc whose two radii close the wedge into a face."""

    kind = "pie"
    shortcut = None


from tools.scale_host import ScaleHost


class ScaleTool(ScaleHost, RustTool):
    kind = "scale"
    uses_snap = ScaleHost.uses_snap

from tools.rotate_host import RotateHost


class RotateTool(RotateHost, RustTool):
    kind = "rotate"
    _READ_DURING = ("start_point", "ref_point", "copy", "array_token")

    def __init__(self):
        RustTool.__init__(self)
        RotateHost.__init__(self)

    # The shared protractor clears its arms at reset. Inside a machine
    # callback the machine already did that; direct setters let Move
    # start a rotation from a selected group's grip.
    def _set_arm(self, key, point):
        if self._during is None:
            self._machine.set(key, _tuple(point))

    start_point = property(lambda self: self._point("start_point"),
                           lambda self, p: self._set_arm("start_point", p))
    ref_point = property(lambda self: self._point("ref_point"),
                         lambda self, p: self._set_arm("ref_point", p))
    _copy = property(lambda self: bool(self._state("copy")),
                     lambda self, on: self._machine.set("copy", bool(on)))
    _last = property(lambda self: True if self._state("retype") else None)


from tools.walkthrough_host import (EyeHost, PositionCameraHost, WalkHost, FirstPersonHost,
                                    DEFAULT_EYE_HEIGHT, DEFAULT_LOOK_SENSITIVITY, eye_height,
                                    set_eye_height, look_sensitivity, look_deg_per_px,
                                    _NO_WALK_REACH, _blocked)


class PositionCameraTool(PositionCameraHost, RustTool):
    kind = "position_camera"


class LookAroundTool(EyeHost, RustTool):
    kind = "look_around"
    TURN_DEG_PER_WIDTH = 180.
    PITCH_DEG_PER_HEIGHT = 90.


class WalkTool(WalkHost, RustTool):
    kind = "walk"


class FirstPersonTool(FirstPersonHost, RustTool):
    kind = "first_person"

from tools.offset_host import OffsetHost, _run_from_edge, _run_from_edges


class OffsetTool(OffsetHost, RustTool):
    kind = "offset"
    uses_snap = OffsetHost.uses_snap


from tools.paint_host import (PaintHost, DEFAULT_FACE_COLOR, RECENT_MAX, recent_key,
                              push_recent, rename_recent, clicked_back_side)
from core.paint_ops import _face_plane, _surface_commands, _texture_commands


class PaintTool(PaintHost, RustTool):
    kind = "paint"

from tools.protractor_host import ProtractorHost, ProtractorBase, DISC_PX, TICK_DEG


class ProtractorTool(ProtractorHost, RustTool):
    kind = "protractor"
    _READ_DURING = ("start_point", "ref_point", "guides")

    def __init__(self):
        RustTool.__init__(self)
        ProtractorBase.__init__(self)

    def _set_arm(self, key, p):
        if self._during is None:
            self._machine.set(key, _tuple(p))

    start_point = property(lambda self: self._point("start_point"),
                           lambda self, p: self._set_arm("start_point", p))
    ref_point = property(lambda self: self._point("ref_point"),
                         lambda self, p: self._set_arm("ref_point", p))
    _guides = property(lambda self: self._state("guides"),
                       lambda self, g: self._machine.set("guides", bool(g)))
    _last = property(lambda self: True if self._state("retype") else None)

from tools.fillet_host import FilletHost


class FilletTool(FilletHost, RustTool):
    kind = "fillet"
    _READ_DURING = ("sizing",)
    sizing = property(lambda self: self._state("sizing"))

    def __init__(self):
        RustTool.__init__(self)
        FilletHost.__init__(self)


from tools.tape_host import TapeHost


class TapeMeasureTool(TapeHost, RustTool):
    kind = "tape"
    _READ_DURING = ("start_point", "hover_point", "mode", "direction")
    start_point = _point_property("start_point")
    hover_point = _point_property("hover_point")
    _mode = property(lambda self: self._state("mode"),
                     lambda self, value: self._machine.set("mode", value))
    _measured = property(lambda self: self._state("measured"))

    @property
    def _edge(self):
        return self._source if self._state("direction") is not None else None

    @_edge.setter
    def _edge(self, edge):
        self._source = edge
        self._machine.set("direction", _tuple(edge.b - edge.a) if edge is not None else None)
