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
from ingetrazo_core import register_tool_adapter, register_tool_functions

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


@register_tool_adapter
class _PlanarMachine(PlaneLock, RustTool):
    """Common event, projection and lock bindings of the planar drawing tools."""
    def __init__(self):
        RustTool.__init__(self)
        self._viewport = None

    start_point = _point_property("start_point")
    hover_point = _point_property("hover_point")

    work_plane = LineTool.work_plane


@register_tool_adapter
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


    def _flag(name):
        def get(self):
            return self._state(name)

        def put(self, value):
            self._machine.set(name, value)
        return property(get, put)

    _live_side = _flag("live_side")
    _arrow_square_lock = _flag("arrow_square")
    _shift_square_lock = _flag("shift_square")


@register_tool_adapter
class _RadialTool(AxisMagnet, _PlanarMachine):
    """Circle and Polygon (``rust/ingetrazo_core/src/tools/circle.rs``): a
    centre, then a radius. The machine owns the plane lock, the side count,
    the guide mode and the retype window; this keeps the viewport glue (the
    status, the axis colour, the cursor's ring, the readout)."""

    vcb_label = "Radius"
    #: Whether Ctrl toggles the guide-circle mode (only the Circle tool).
    _guide_capable = False
    wireframe_color = None


class CircleTool(_RadialTool):
    kind = "circle"
    _guide_capable = True


class PolygonTool(_RadialTool):
    kind = "polygon"
    shortcut = None  # Polygon has no default; G = Make Component


@register_tool_adapter
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




# ---- Arcs ---------------------------------------------------------------------

#: Magenta of the arc tangent to BOTH edges of a corner, the fillet.
COLOR_FILLET = (0.85, 0.30, 0.80)


@register_tool_adapter
class _ArcBase(AxisMagnet, _PlanarMachine):
    """The arcs (``rust/ingetrazo_core/src/tools/arc.rs``): the machine owns
    the clicks, the plane lock, the segment count and the commit; this keeps
    the viewport glue (status, colours, readouts)."""

    wireframe_color = None


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


@register_tool_adapter
class ArcTool(_ArcBase, metaclass=_LastFillet):
    """2-point arc. The host answers the topology the fillet reads (the
    ``host_*`` queries) and projects points to pixels."""

    kind = "arc"
    vcb_label = "Bulge"

    end_point = _point_property("end_point")

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

@register_tool_adapter
class ThreePointArcTool(_ArcBase):
    """3-point arc: the circle through the three clicks."""

    kind = "arc3"
    vcb_label = "Segments"

    mid_point = _point_property("mid_point")


@register_tool_adapter
class CenterArcTool(_ArcBase):
    """Centre arc: centre, the 0° arm, the sweep."""

    kind = "center_arc"
    vcb_label = "Angle"

    arm_point = _point_property("arm_point")


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



register_tool_functions(globals())
