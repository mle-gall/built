# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Resident bevel targets, preview mesh and panel bindings for the Fillet machine."""
from __future__ import annotations

from core.input_types import Qt
from ingetrazo_core import Vec3 as QVector3D, bevel_nearest_end, bevel_selection, bevel_targets

from core.fillet import CIRCULAR, plan_fillet
from core.units import fmt_len_fine
from core.i18n import tr
from core.mesh import Edge
from core.vertex_bevel import plan_vertex_bevel
from tools.base import Tool


class FilletHost(Tool):
    icon = "fillet"
    wireframe_color = (0.13, 0.17, 0.23, 1.0)
    wireframe_depth_tested = True

    #: Last radius used — the next fillet starts from it (metres).
    radius: float = 0.10
    #: Arc segments of the strip.
    segments: int = 8
    #: Shape of the section — 0.5 is the circular arc (see core.fillet).
    profile: float = CIRCULAR
    #: How the size is read; Ctrl walks them in this order.
    measure: str = "radius"
    _MEASURES = (("radius", "Radius"), ("offset", "Distance"),
                 ("width", "Width"), ("depth", "Depth"))
    #: How a box corner closes: "round" (patch) or "cut" (flat).
    corners: str = "round"
    #: A drawn outline instead of the superellipse (core.bevel_profiles):
    #: a preset's name, the editor's control points (a tuple), or None.
    shape = None
    _SHAPES = (("steps", "Steps"), ("supports", "Round with supports"),
               ("cornice", "Cornice"), ("crown", "Crown moulding"))
    #: What is bevelled: "edges", or "vertices" (the corners alone, the
    #: size then running along their edges).
    affect: str = "edges"

    def __init__(self) -> None:
        self.edges: list = []
        self.hover_edge = None
        self.hover_vertex = None
        self.hover_point: QVector3D | None = None
        self._plan = None
        self._plan_key = None
        self._message: str | None = None
        self._viewport = None

    def _corners_mode(self) -> bool:
        return self.affect == "vertices"

    # ---- Lifecycle ----------------------------------------------------------
    def host_reset_bevel(self, viewport, activate):
        self._viewport = viewport
        self._reset()
        if not activate:
            if hasattr(viewport, "set_hover"):
                viewport.set_hover(None)
            return
        scene = getattr(viewport, "scene", None)
        sel = getattr(scene, "selection", None) or ()
        mesh = getattr(scene, "mesh", None)
        picked = bevel_selection(sel, mesh, self._corners_mode())
        if picked and self._corners_mode():
            self.edges = picked
            viewport.flash_status(tr(
                "{n} corner(s) selected — type the size, or click and move "
                "the cursor to set it.", n=len(picked)), 5000)
        elif picked:
            self.edges = picked
            viewport.flash_status(tr(
                "{n} edge(s) selected — type the size, or click and move "
                "the cursor to set it.", n=len(picked)), 5000)
        else:
            viewport.flash_status(tr(
                "Click an edge to round or bevel it; type the size and Enter."), 4000)
        self._replan()

    def _reset(self) -> None:
        self.edges = []
        self.hover_edge = None
        self.hover_vertex = None
        self.hover_point = None
        self._plan = None
        self._plan_key = None
        self._message = None

    _nearest_end = staticmethod(bevel_nearest_end)

    # ---- Spatial input --------------------------------------------------------
    @property
    def start_point(self):
        """Esc cascade hook: an in-progress rounding to cancel."""
        if not self.edges:
            return None
        first = self.edges[0]
        return first.a if isinstance(first, Edge) else first.position

    def host_pick_bevel(self, vp, world):
        self._viewport = vp
        point = QVector3D(*world) if world is not None else None
        if not self.edges:
            self.edges = bevel_targets(vp, self._corners_mode(), point)
        if self.edges:
            self.hover_point = point
        return bool(self.edges)

    def host_hover_bevel(self, vp, world):
        if self.edges:
            return
        edge = getattr(vp, "_hover_edge", None)
        self.hover_edge = edge if isinstance(edge, Edge) else None
        self.hover_vertex = (self._nearest_end(self.hover_edge, self.hover_point)
                             if self.hover_edge is not None else None)
        if hasattr(vp, "set_hover"):
            vp.set_hover(self.hover_edge)
        self._replan()

    def on_hover(self, ctx):
        self._viewport = ctx.viewport
        self.hover_point = ctx.world
        self._send(ctx.viewport, self._pointer("hover", ctx))
        ctx.viewport.update()

    def host_bevel_geometry(self, vp, _):
        return [[e.a.toTuple(), e.b.toTuple()] if isinstance(e, Edge) else [e.position.toTuple()]
                for e in self.edges]

    def host_replan(self, vp, _):
        self._replan()

    def host_bevel_picked(self, vp, _):
        return bool(self.edges)

    def on_key(self, viewport, key, modifiers):
        return key == Qt.Key_Control and self._send(viewport, {"type": "key", "key": "Control"})["consumed"]

    # ---- What is bevelled -------------------------------------------------------
    def set_affect(self, viewport, affect: str) -> None:
        """Edges or vertices. Picked edges carry over as their corners;
        picked corners do not make edges, so they are let go."""
        if affect not in ("edges", "vertices") or affect == self.affect:
            return
        type(self).affect = affect
        mesh = getattr(getattr(viewport, "scene", None), "mesh", None)
        if affect == "vertices":
            type(self).shape = None      # outlines are drawn along edges
        if affect == "vertices" and self.edges:
            self.edges = bevel_selection(self.edges, mesh, True)
        elif affect == "edges":
            self.edges = []
            self._send(viewport, {"type": "key", "key": "AffectEdges"})
        self.hover_vertex = None
        viewport.flash_status(tr("Affect: {a}", a=tr(
            "Vertices" if affect == "vertices" else "Edges")))
        self._replan()
        self._refresh_readouts(viewport)
        viewport.update()

    def _refresh_readouts(self, viewport) -> None:
        """The status clause and the VCB caption name the mode: redraw them
        now, not on the next live value (with nothing picked there is
        none)."""
        hint = getattr(viewport, "refresh_status_hint", None)
        if hint is not None:
            hint()
        window = getattr(viewport, "window", None)
        vcb = getattr(window(), "_refresh_vcb", None) if callable(window) else None
        if vcb is not None:
            vcb()

    def on_key_release(self, viewport, key: int) -> bool:
        return key == Qt.Key_Control and self._send(viewport, {
            "type": "key_release", "key": "Control",
            "tapped": getattr(viewport, "ctrl_tapped", lambda: True)()})["consumed"]

    def host_bevel_measure(self, viewport, _):
        return None if self._corners_mode() else self.measure

    def host_measure(self, viewport, measure):
        self.set_measure(viewport, measure)

    def set_measure(self, viewport, measure: str) -> None:
        if measure not in dict(self._MEASURES):
            return
        type(self).measure = measure
        viewport.flash_status(tr("Measure: {m}", m=tr(self._measure_label())))
        self._replan()
        self._refresh_readouts(viewport)
        viewport.update()

    def set_size(self, viewport, value: float) -> None:
        """The panel's Amount: the size, without rounding yet."""
        type(self).radius = float(value)
        self._replan()
        viewport.update()

    def set_shape(self, viewport, shape) -> None:
        """A drawn outline (steps, mouldings…), or None for the
        superellipse."""
        from core import bevel_profiles
        if isinstance(shape, (list, tuple)):
            if not bevel_profiles.valid(shape):
                return
            shape = tuple((float(x), float(y), bool(sm)) for x, y, sm in shape)
        elif shape is not None and shape not in dict(self._SHAPES):
            return
        if shape is not None and self._corners_mode():
            viewport.flash_status(tr("Drawn profiles bevel edges only."))
            return
        type(self).shape = shape
        if isinstance(shape, str):
            viewport.flash_status(tr("Profile {p}", p=tr(self._shape_label())))
        self._replan()
        viewport.update()

    def set_corners(self, viewport, corners: str) -> None:
        if corners not in ("round", "cut"):
            return
        type(self).corners = corners
        viewport.flash_status(tr("Corners: {c}", c=tr(
            "Fill (triangle / cap)" if corners == "round" else "No fill (notch)")))
        self._replan()
        viewport.update()

    def apply_from_panel(self, viewport) -> None:
        """The panel's Apply: round what is picked, as Enter would."""
        if not self._current_edges():
            viewport.flash_status(tr("Click an edge to round or bevel it; "
                                     "type the size and Enter."), 4000)
            return
        self._send(viewport, {"type": "key", "key": "Apply"})

    def _measure_label(self) -> str:
        """English source label of the current measure."""
        return dict(self._MEASURES).get(self.measure, "Radius")

    def vcb_caption(self) -> str:
        return "Distance" if self._corners_mode() else self._measure_label()

    def status_clause(self) -> str:
        if self._corners_mode():
            return tr("Vertices: size along the edges")
        return tr("Ctrl = measure ({m})", m=tr(self._measure_label()))

    def on_value(self, viewport, value):
        result = self._send(viewport, {"type": "value", "value": value})
        if result["redraw"]:
            viewport.update()
        return result["consumed"]

    def on_segments_value(self, viewport, n):
        answer = self._send(viewport, {"type": "segments", "count": int(n)})
        if answer["redraw"]:
            viewport.update()
        return answer["consumed"]

    def on_profile_value(self, viewport, p):
        answer = self._send(viewport, {"type": "profile", "value": p})
        if answer["redraw"]:
            viewport.update()
        return answer["consumed"]

    def host_panel_value(self, viewport, args):
        field, value = args
        setattr(type(self), field, value)
        if field == "segments":
            viewport.flash_status(tr("{n} segments", n=value))
        elif field == "profile":
            type(self).shape = None
            viewport.flash_status(tr("Profile {p}", p=f"{value:.2f}"))

    def host_bad_profile(self, viewport, _):
        viewport.flash_status(tr("The profile must be between 0 and 1."))

    def host_announce_size(self, viewport, _):
        viewport.flash_status(tr("{m} {r} — click an edge.",
                                 m=tr(self._measure_label()), r=fmt_len_fine(self.radius)), 3000)

    # ---- Planning / commit ----------------------------------------------------
    def _current_edges(self) -> list:
        """What would be bevelled now: the picked edges (or corners), else
        the one under the cursor."""
        if self.edges:
            return self.edges
        if self._corners_mode():
            return [self.hover_vertex] if self.hover_vertex is not None else []
        return [self.hover_edge] if self.hover_edge is not None else []

    def _replan(self) -> None:
        edges = self._current_edges()
        mesh = getattr(getattr(self._viewport, "scene", None), "mesh", None)
        if not edges or mesh is None:
            self._plan = None
            self._plan_key = None
            self._message = None
            return
        key = (tuple(e.key for e in edges), round(self.radius, 6),
               self.segments, self.profile, self.measure,
               self.corners, self.affect, self.shape,
               getattr(mesh, "_mut_serial", 0),
               getattr(getattr(mesh, "_store", None), "revision", None))
        if key == self._plan_key:
            return
        self._plan_key = key
        if self._corners_mode():
            plan = plan_vertex_bevel(mesh, edges, self.radius,
                                     self.segments, self.profile)
        else:
            plan = plan_fillet(mesh, edges, self.radius,
                               self.segments, self.profile,
                               self.measure, self.corners,
                               self.shape)
        if isinstance(plan, str):
            self._plan = None
            self._message = plan
        else:
            self._plan = plan
            self._message = None

    def host_fillet_plan(self, viewport, _):
        self._replan()
        if self._plan is None:
            viewport.flash_status(self._message or tr("Nothing to round."), 4000)
            return None
        from core.protocol.keys import to_json_key
        kind = "vertices" if self._corners_mode() else "edges"
        return {kind: [to_json_key(e.key) for e in self._current_edges()],
                "radius": self.radius, "segments": self.segments, "profile": self.profile,
                "measure": self.measure, "corners": self.corners, "shape": self.shape}

    def host_finish_bevel(self, viewport, _):
        n, r = len(self._current_edges()), self.radius
        viewport.scene.clear_selection()
        self._reset()
        set_hover = getattr(viewport, "set_hover", None)
        if set_hover is not None:
            set_hover(None)
        if self._corners_mode():
            said = tr("Bevelled {n} corner(s), distance {r}.", n=n,
                      r=fmt_len_fine(r))
        elif self._is_plain_fillet():
            said = tr("Rounded {n} edge(s), radius {r}.", n=n,
                      r=fmt_len_fine(r))
        else:
            said = tr("Bevelled {n} edge(s), {m} {r}.", n=n,
                      m=tr(self._measure_label()), r=fmt_len_fine(r))
        viewport.flash_status(said, 3000)
        viewport.update()

    # ---- Preview --------------------------------------------------------------
    def preview_faces(self):
        if self._plan is None:
            return []
        from core.geometry import Face as PreviewFace
        return [PreviewFace(list(loop)) for loop in self._plan.new_faces]

    def rubber_band_lines(self):
        if self._plan is None:
            return []
        lines = []
        for pts in self._plan.curves:
            lines.extend((pts[i], pts[i + 1]) for i in range(len(pts) - 1))
        return lines

    def value_label(self):
        edges = self._current_edges()
        if not edges:
            return None
        e = edges[0]
        mid = (e.a + e.b) * 0.5 if isinstance(e, Edge) else QVector3D(e.position)
        if self._message:
            return (self._message, mid)
        size = fmt_len_fine(self.radius)
        if self._corners_mode():
            head = tr("Distance") + " " + size
        elif self.measure == "radius":
            head = "R " + size
        else:
            head = tr(self._measure_label()) + " " + size
        extra = tr("{n} segments", n=self.segments)
        if self.shape is not None and not self._corners_mode():
            extra += ", " + tr(self._shape_label())
        elif abs(self.profile - CIRCULAR) > 1e-9:
            extra += ", " + tr("profile {p}", p=f"{self.profile:.2f}")
        return (head + "  (" + extra + ")", mid)

    def _shape_label(self) -> str:
        """English source label of the drawn outline in use."""
        if isinstance(self.shape, tuple):
            return "Custom (draw)"
        return dict(self._SHAPES).get(self.shape, "Curve (value 0–1)")

    def _is_plain_fillet(self) -> bool:
        return (self.measure == "radius" and self.shape is None
                and abs(self.profile - CIRCULAR) < 1e-9)
