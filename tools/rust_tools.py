# SPDX-License-Identifier: GPL-3.0-or-later
"""The Rust tools seen from Python (docs/rust-tools.md §10).

Each class keeps the name, attributes and methods the hosts, the inference
and extensions use (``on_click(ctx)``, ``rubber_band_lines()``,
``start_point``…), turns the call into an event for its
``ingetrazo_core.ToolMachine`` and answers the machine's host calls. The
native viewport host handles picks, document queries and protocol commands.
Python retains tool identities for extensions, UI readouts and view callbacks.
"""
from __future__ import annotations
from ingetrazo_core import register_tool_adapter

import json

from ingetrazo_core import ToolMachine, Vec3
from tools.rust_tool_base import RustTool, _Host, _mods, _point_property, _tuple, selection_mode


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


@register_tool_adapter
class SelectTool(RustTool):
    """Select (``rust/ingetrazo_core/src/tools/select.rs``). ``press``,
    ``drag``, ``release`` and ``double`` are the raw gesture the Metal view
    sends; the ``on_*`` methods are the older, already-decided entry points."""

    kind = "select"

    _MODE_MODS = {"replace": 0, "add": 0x04000000, "toggle": 0x02000000, "remove": 0x06000000}


from tools.base import AxisMagnet, PlaneLock


class _PlanarMachine(PlaneLock, RustTool):
    """Common event, projection and lock bindings of the planar drawing tools."""
    def __init__(self):
        RustTool.__init__(self)
        self._viewport = None

    @property
    def _plane_state(self):
        return self._machine

    start_point = _point_property("start_point")
    hover_point = _point_property("hover_point")

    work_plane = LineTool.work_plane

    def _frame(self, viewport):
        from core import axes
        from core.snap import POINT_SNAP_KINDS
        kind = getattr(self, "snap_kind", None)
        self._send(viewport, {"type": "frame", "axes": axes.frame(),
                              "names_point": None if kind is None else kind in POINT_SNAP_KINDS})

    def _send_framed(self, viewport, event):
        self._frame(viewport)
        return self._send(viewport, event)

    def _cursor_preview(self):
        vp, point = self._viewport, self.hover_point
        if vp is None or point is None:
            return []
        from core.axes import plane_axes
        _, normal = self.preview_plane(vp, point)
        u, v = plane_axes(normal)
        scale = self.plane_preview_scale(vp, point, u, v)
        if scale is None:
            return []
        return self._machine.cursor_lines(point, (u.toTuple(), v.toTuple()), self.PREVIEW_PX * scale)

    def rubber_band_lines(self):
        return self._cursor_preview() if self.start_point is None else super().rubber_band_lines()


class RectangleTool(_PlanarMachine):
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


    @property
    def chain_first_point(self):
        return None             # no close-polygon snap onto the first corner

    @chain_first_point.setter
    def chain_first_point(self, _value):
        pass

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
        from core.input_types import Qt
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
        from core.input_types import Qt
        if key != Qt.Key_Control:
            return False
        tapped = getattr(viewport, "ctrl_tapped", None)
        reply = self._send_framed(viewport, {"type": "key_release", "key": "Control",
                                             "tapped": not callable(tapped) or bool(tapped())})
        if reply["consumed"]:
            self._set_from_center(viewport, self._from_center)
        return reply["consumed"]

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


class _RadialTool(AxisMagnet, _PlanarMachine):
    """Circle and Polygon (``rust/ingetrazo_core/src/tools/circle.rs``): a
    centre, then a radius. The machine owns the plane lock, the side count,
    the guide mode and the retype window; this keeps the viewport glue (the
    status, the axis colour, the cursor's ring, the readout)."""

    vcb_label = "Radius"
    #: Whether Ctrl toggles the guide-circle mode (only the Circle tool).
    _guide_capable = False
    wireframe_color = None


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
        from core.input_types import Qt
        if key == Qt.Key_Control and self._guide_capable:
            self._send_framed(viewport, {"type": "key", "key": "Control", "mods": _mods(modifiers)})
            self._refresh(viewport)
            return True
        return self.plane_lock_key(viewport, key)

    def on_key_release(self, viewport, key) -> bool:
        from core.input_types import Qt
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


class RotatedRectangleTool(AxisMagnet, _PlanarMachine):
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

    base_point = _point_property("base_point")

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


class _ArcBase(AxisMagnet, _PlanarMachine):
    """The arcs (``rust/ingetrazo_core/src/tools/arc.rs``): the machine owns
    the clicks, the plane lock, the segment count and the commit; this keeps
    the viewport glue (status, colours, readouts)."""

    wireframe_color = None


    @property
    def segments(self):
        return self._state("segments")

    @segments.setter
    def segments(self, n):
        self._machine.set("segments", None if n is None else int(n))

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
        t = _Host(viewport, self._handles, self).arc_tangent(self.start_point.toTuple())
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


@register_tool_adapter
class RotateTool(RotateHost, RustTool):
    kind = "rotate"
    _READ_DURING = ("start_point", "ref_point", "copy", "array_token")

    def __init__(self):
        RustTool.__init__(self)
        RotateHost.__init__(self)

    # The shared protractor clears its arms at reset. Inside a machine
    # callback the machine already did that; direct setters let Move
    # start a rotation from a selected group's grip.
    start_point = property(lambda self: self._point("start_point"),
                           lambda self, p: self._set_arm("start_point", p))
    ref_point = property(lambda self: self._point("ref_point"),
                         lambda self, p: self._set_arm("ref_point", p))
    _copy = property(lambda self: bool(self._state("copy")),
                     lambda self, on: self._machine.set("copy", bool(on)))
    _last = property(lambda self: True if self._state("retype") else None)


from tools.walkthrough_host import (EyeHost, PositionCameraHost, WalkHost, FirstPersonHost,
                                    DEFAULT_EYE_HEIGHT, DEFAULT_LOOK_SENSITIVITY, eye_height,
                                    set_eye_height, look_sensitivity, look_deg_per_px)


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

from tools.offset_host import OffsetHost


class OffsetTool(OffsetHost, RustTool):
    kind = "offset"
    uses_snap = OffsetHost.uses_snap


from tools.paint_host import (PaintHost, DEFAULT_FACE_COLOR, RECENT_MAX, recent_key,
                              push_recent, rename_recent, clicked_back_side)


class PaintTool(PaintHost, RustTool):
    kind = "paint"

from tools.protractor_host import ProtractorHost, ProtractorBase, DISC_PX, TICK_DEG


@register_tool_adapter
class ProtractorTool(ProtractorHost, RustTool):
    kind = "protractor"
    _READ_DURING = ("start_point", "ref_point", "guides")

    def __init__(self):
        RustTool.__init__(self)
        ProtractorBase.__init__(self)

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
    sizing = property(lambda self: self._state("sizing"),
                      lambda self, value: self._machine.set("sizing", bool(value)))

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

from tools.paste_host import PasteHost


class PasteTool(PasteHost, RustTool):
    kind = "paste"
    shortcut = None
    description = None
    start_point = _point_property("start_point")
    _offset = _point_property("offset")


from tools.flip_host import FlipHost


class FlipTool(FlipHost, RustTool):
    kind = "flip"
    shortcut = None
    _READ_DURING = ("hover_point", "hover_axis", "lock_axis", "copy")
    hover_point = _point_property("hover_point")
    _lock_axis = property(lambda self: self._state("lock_axis"),
                          lambda self, axis: self._machine.set("lock_axis", axis))
    _hover_axis = property(lambda self: self._state("hover_axis"),
                           lambda self, axis: self._machine.set("hover_axis", axis))
    _copy = property(lambda self: self._state("copy"),
                     lambda self, value: self._machine.set("copy", bool(value)))
    cursor_plus = _copy


class FreehandTool(RustTool):
    kind = "freehand"
    shortcut = None
    work_plane = None
    start_point = property(lambda self: self._point("start_point"))
    _stroke = property(lambda self: self._state("stroke"))

    def _clear(self, viewport, event):
        self.work_plane = None
        self._send(viewport, {"type": event})

    def on_activate(self, viewport):
        self._clear(viewport, "activate")

    def on_deactivate(self, viewport):
        self._clear(viewport, "deactivate")

    def on_cancel(self, viewport):
        self._clear(viewport, "cancel")
        viewport.update()

    def on_release(self, viewport):
        self._clear(viewport, "release")
        viewport.update()

    def on_hover(self, ctx):
        if self._send(ctx.viewport, self._pointer("hover", ctx))["redraw"]:
            ctx.viewport.update()


class ChangeAxesTool(RustTool):
    kind = "change_axes"
    shortcut = None
    description = None
    target = None
    start_point = _point_property("start_point")
    red_point = _point_property("red_point")
    hover_point = _point_property("hover_point")

    def host_axes_target(self, vp, _):
        return self.target.uid if self.target is not None else None

    def host_axes_message(self, vp, key):
        from core.i18n import tr
        if key == "collinear":
            vp.flash_status(tr("The green direction cannot run along red — click off the red axis"), 4000)
        elif key == "done":
            vp.flash_status(tr("Axes changed"), 2500)
        else:
            labels = {"empty": "Select one group or component first", "red": "Click along the new red axis",
                      "green": "Click to set the green direction"}
            vp.flash_status(tr(labels[key]))

    def on_click(self, ctx):
        vp = ctx.viewport
        if self._send(vp, self._pointer("click", ctx))["committed"]:
            self.target = None
            window = vp.window() if hasattr(vp, "window") else None
            if hasattr(window, "_activate_tool"):
                window._activate_tool("select")
        vp.update()

    def value_label(self):
        from core.i18n import tr
        if self.start_point is not None and self.hover_point is not None:
            return (tr("Red axis") if self.red_point is None else tr("Green axis")), self.hover_point


from tools.followme_host import FollowMeHost


class FollowMeTool(FollowMeHost, RustTool):
    kind = "followme"


from tools.solid_host import (SolidTool, OuterShellTool, UnionTool, SubtractTool, TrimTool, IntersectTool, SplitTool, SOLID_TOOLS)


from tools.scale_manager_host import ScaleManagerTool


from tools.image_host import ImageTool


from tools.north_host import (NorthTool, COMPASS_PX, refresh_north_display, north_deg_towards, compass_segments)


from tools.section_host import SectionPlaneTool


from tools.annotation_host import TextTool, DimensionTool


from tools.geopath_host import GeoPathTool


from tools.eraser_host import EraserHost


class EraserTool(EraserHost, RustTool):
    kind = "eraser"


from tools.place_group_host import PlaceGroupHost


class PlaceGroupTool(PlaceGroupHost, RustTool):
    kind = "place_group"
    shortcut = description = None
    _face_normal = _point_property("normal")
    _offset = property(lambda self: self._point("offset"))


from tools.move_host import MoveHost, rotation_grips, gather_targets, gather_images


class MoveTool(MoveHost):
    kind = "move"


def commit_arc(viewport, pts, close_to=None, trim=None):
    """Draw an arc polyline (Freehand's commit): the protocol's ``add_arc``."""
    args = {"points": [list(_tuple(p)) for p in pts]}
    if close_to is not None:
        args["center"] = list(_tuple(close_to))
    if trim:
        args["trim"] = [[list(_tuple(a)), list(_tuple(b))] for a, b in trim]
    return _Host(viewport).run("add_arc", json.dumps(args))
