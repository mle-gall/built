# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Scale grip picking and resident-row previews for the Rust machine."""
from __future__ import annotations

from PySide6.QtCore import Qt
from ingetrazo_core import Matrix4 as QMatrix4x4, ScaleGripSpec, Vec3 as QVector3D

from core.history import scale_matrix
from core.i18n import tr


# Below this extent an axis is flat: it gets no grips and never scales
# (a 2D face shows the 8-grip box instead of a degenerate 26).
_FLAT = 1e-6
_AXIS_NAMES = ("Red", "Green", "Blue")     # X east, Y north, Z up (Z-up)


class _Grip:
    """One scaling grip: box-parameter position + the axes it scales."""

    __slots__ = ("params", "mask")

    def __init__(self, params: tuple, mask: tuple) -> None:
        self.params = params               # (tx, ty, tz), each 0 / 0.5 / 1
        self.mask = mask                   # axis indices this grip scales

    def kind(self, active_axes: int) -> str:
        n = len(self.mask)
        if n >= active_axes:
            return "corner"
        return "edge" if n == 2 else "face"


class ScaleHost:
    @property
    def uses_snap(self) -> bool:
        """Only while a grip is held: then a corner, a midpoint or an edge
        of ANOTHER object sets the size — scale «to» it (issue #233,
        @ales-limon). Before the grab, the grips take the click, not the
        geometry."""
        return self._grip is not None

    # The VCB tags unit-suffixed entries for us ("2m" = absolute size), so a
    # bare "2" can mean ×2, the usual reading.
    accepts_absolute_length = True

    def __init__(self) -> None:
        super().__init__()
        # Box + grips, in the coordinates of the box's frame (see
        # ``_pick_frame``): the world's unless the selection has its own.
        self._lo: QVector3D | None = None
        self._hi: QVector3D | None = None
        self._axes = None                  # (red, green, blue) or None = world
        self._orig = None
        self._grips: list[_Grip] = []
        self._box_version = -1
        # Targets resolved at grab time.
        self._groups: list = []
        self._positions: list[QVector3D] = []
        self._verts: list = []
        self._images: list = []
        # The operation in flight.
        self._grip: _Grip | None = None
        self._hover_grip: _Grip | None = None
        self._anchor: QVector3D | None = None
        self._factors = (1.0, 1.0, 1.0)    # live preview factors
        # A tap toggles.
        self.about_center = False
        self.uniform = False

    # ---- Lifecycle ----------------------------------------------------------
    def host_activate_box(self, viewport, _) -> None:
        self._reset()
        self.about_center = False
        self.uniform = False
        self._refresh_box(viewport)

    def host_deactivate_box(self, viewport, _) -> None:
        self._revert_preview(viewport)
        self._reset()

    # ---- Box + grips --------------------------------------------------------
    def _selection_bounds(self, viewport):
        """World AABB of the selection: loose vertices, whole groups (their
        nested placements included) and reference images."""
        from core.group import iter_placements
        from core.image_plane import ImagePlane
        from core.mesh import Edge, Face

        self._pick_frame(viewport)
        lo = [float("inf")] * 3
        hi = [float("-inf")] * 3
        seen = False

        def absorb(p: QVector3D) -> None:
            nonlocal seen
            seen = True
            p = self._to_local(p)
            for i, c in enumerate((p.x(), p.y(), p.z())):
                if c < lo[i]:
                    lo[i] = c
                if c > hi[i]:
                    hi[i] = c

        for ent in viewport.scene.selection:
            if isinstance(ent, Edge):
                absorb(ent.a)
                absorb(ent.b)
            elif isinstance(ent, Face):
                for v in ent.vertices:
                    absorb(v)
            elif isinstance(ent, ImagePlane):
                for c in ent.corners():
                    absorb(c)
            elif hasattr(ent, "mesh"):     # Group / component instance
                for pg, m in iter_placements(ent):
                    for v in pg.mesh.vertices:
                        p = m.map(v.position) if m is not None else v.position
                        absorb(p)
        if not seen:
            return None, None
        return QVector3D(*lo), QVector3D(*hi)

    def _refresh_box(self, viewport) -> None:
        """(Re)build the grip box from the selection — cheap to call: keyed on
        ``scene.version``, which selection changes already bump."""
        if self._grip is not None:
            return                          # never while an operation is live
        if viewport.scene.version == self._box_version:
            return
        self._box_version = viewport.scene.version
        self._lo, self._hi = self._selection_bounds(viewport)
        self._grips = []
        self._hover_grip = None
        if self._lo is None:
            return
        from ingetrazo_core import scale_box_geometry
        grips, self._box_edges = scale_box_geometry(self._extents())
        self._grips = [_Grip(tuple(p), tuple(mask)) for p, mask in grips]
        if not grips:
            self._lo = self._hi = None

    def _extents(self) -> tuple:
        return (self._hi.x() - self._lo.x(),
                self._hi.y() - self._lo.y(),
                self._hi.z() - self._lo.z())

    def _active_axes(self) -> int:
        ext = self._extents()
        return sum(1 for i in range(3) if ext[i] > _FLAT)

    # ---- The box's frame (issue #44) ----------------------------------------
    def _pick_frame(self, viewport) -> None:
        """One group or component selected: its own axes. Anything else:
        the drawing axes. The world frame is kept as ``None`` so every
        number is exactly what it was before the box could turn."""
        from core import axes
        from core.group import Group, frame_axes, group_frame
        sel = list(viewport.scene.selection)
        frame = None
        if len(sel) == 1 and isinstance(sel[0], Group):
            frame = group_frame(sel[0])
        if frame is None and not axes.is_world():
            frame = axes.frame_matrix(axes.ORIGIN, *[axes.AXES[k]
                                                     for k in "xyz"])
        if frame is None:
            self._axes, self._orig = None, None
            return
        o, x, y, z = frame_axes(frame)
        if (x, y, z) == (QVector3D(1, 0, 0), QVector3D(0, 1, 0),
                         QVector3D(0, 0, 1)):
            self._axes, self._orig = None, None
            return
        self._axes, self._orig = (x, y, z), o

    def _to_local(self, p: QVector3D) -> QVector3D:
        if self._axes is None:
            return QVector3D(p)
        d = p - self._orig
        x, y, z = self._axes
        return QVector3D(QVector3D.dotProduct(d, x), QVector3D.dotProduct(d, y),
                         QVector3D.dotProduct(d, z))

    def _to_world(self, p: QVector3D) -> QVector3D:
        if self._axes is None:
            return QVector3D(p)
        x, y, z = self._axes
        return self._orig + x * p.x() + y * p.y() + z * p.z()

    def _grip_pos(self, grip: _Grip) -> QVector3D:
        lo, hi = self._lo, self._hi
        t = grip.params
        return self._to_world(QVector3D(lo.x() + (hi.x() - lo.x()) * t[0],
                                        lo.y() + (hi.y() - lo.y()) * t[1],
                                        lo.z() + (hi.z() - lo.z()) * t[2]))

    def _anchor_for(self, grip: _Grip) -> QVector3D:
        """The point straight opposite the grip — or the box centre
        while About Center is toggled on."""
        if self.about_center:
            return self._to_world((self._lo + self._hi) * 0.5)
        params = tuple(1.0 - t if i in grip.mask else 0.5
                       for i, t in enumerate(grip.params))
        return self._grip_pos(_Grip(params, grip.mask))

    #: Half the side of a drawn grip square, in pixels.
    _GRIP_HALF_PX = 5.0

    def _grip_under(self, viewport, sx: float, sy: float) -> _Grip | None:
        """The grip under the cursor. Where two grips overlap on screen the
        one in front wins — it is the one drawn on top (#833) — otherwise
        the nearest on screen."""
        threshold = float(getattr(viewport, "pick_threshold_px", 10.0))
        cam = getattr(viewport, "camera", None)
        eye = cam.eye() if cam is not None else QVector3D()
        fwd = cam.target - eye if cam is not None else QVector3D()
        best, best_key = None, None
        for g in self._grips:
            pos = self._grip_pos(g)
            px = viewport._world_to_pixel(pos)
            if px is None:
                continue
            d = ((px[0] - sx) ** 2 + (px[1] - sy) ** 2) ** 0.5
            if d >= threshold:
                continue
            inside = d <= self._GRIP_HALF_PX
            depth = QVector3D.dotProduct(pos - eye, fwd)
            key = (0, depth) if inside else (1, d)
            if best_key is None or key < best_key:
                best, best_key = g, key
        return best

    # ---- Cursor → factors ---------------------------------------------------
    def _ray(self, viewport, sx: float, sy: float):
        return viewport._pixel_to_ray(sx, sy)

    #: What a held grip may land on: named points and edges, projected onto
    #: the grip's line. Not a face (the grip would jump across every
    #: surface it passes) and not the engine's own directional inferences.
    _TARGET_KINDS = frozenset({
        "endpoint", "midpoint", "intersection", "intersection_edge",
        "intersection_face", "center", "origin",
        "on_edge", "on_line", "from_point", "close", "guide",
        "component_origin", "arc_midpoint", "face_center"})

    def snap_excluded(self):
        """What is being scaled stays out of the snap engine, or the grip
        would land on the very geometry it resizes. ``(edge ids, group
        ids)`` as Move gives it."""
        if self._grip is None:
            return None
        edges = {e.key for v in self._verts for e in getattr(v, "edges", ())}
        return edges, {id(g) for g in self._groups}

    def _snapped_point(self, ctx):
        snap = getattr(ctx, "snap", None) if ctx is not None else None
        if snap is None or snap.kind not in self._TARGET_KINDS:
            return None
        return QVector3D(snap.point)

    def _grip_spec(self):
        grip = self._grip
        return ScaleGripSpec(self._extents(), grip.mask,
                             self.uniform or grip.kind(self._active_axes()) == "corner")

    def _line_grip(self):
        grip = self._grip
        return self.uniform or len(grip.mask) == 1 or grip.kind(self._active_axes()) == "corner"

    def _relative(self, point):
        if self._axes is None or self._line_grip():
            return (point - self._anchor).toTuple()
        return (self._to_local(point) - self._to_local(self._anchor)).toTuple()

    def _factors_from_point(self, p):
        return self._grip_spec().point(self._relative(self._grip_pos(self._grip)),
                                       self._relative(p))

    def _factors_for(self, ctx, viewport, sx: float, sy: float):
        p = self._snapped_point(ctx)
        if p is not None:
            got = self._factors_from_point(p)
            if got is not None:
                return got
        return self._factors_from_cursor(viewport, sx, sy)

    def _factors_from_cursor(self, viewport, sx: float, sy: float):
        origin, direction = self._ray(viewport, sx, sy)
        if origin is None:
            return None
        ray = (direction.toTuple() if self._axes is None or self._line_grip() else
               tuple(QVector3D.dotProduct(direction, axis) for axis in self._axes))
        return self._grip_spec().ray(self._relative(self._grip_pos(self._grip)),
                                     self._relative(origin), ray)

    # ---- Spatial input ------------------------------------------------------
    def _pointer_event(self, kind, ctx):
        self._ctx = ctx
        try:
            self._send(ctx.viewport, self._pointer(kind, ctx))
        finally:
            self._ctx = None

    def on_click(self, ctx):
        self._pointer_event("click", ctx)

    def on_hover(self, ctx):
        self._pointer_event("hover", ctx)

    def host_factors(self, viewport, xy):
        return self._factors_for(self._ctx, viewport, *xy)

    def host_pick_grip(self, viewport, xy):
        from tools.move_host import gather_images, gather_targets
        ctx = self._ctx
        self._refresh_box(viewport)
        sx, sy = xy

        if self._lo is not None:
            grip = self._grip_under(viewport, sx, sy)
            if grip is not None:
                self._capture_grip(viewport, grip, (sx, sy))
                return True

        # No grip hit: with nothing selected, a click picks the entity under
        # the cursor and boxes it (click-to-scale).
        if not viewport.scene.selection:
            groups, positions = gather_targets(ctx)
            images = gather_images(ctx)
            picked = list(groups) + list(images)
            if not picked and positions:
                edge = viewport.pick_edge(sx, sy)
                face = viewport.pick_face(sx, sy) if edge is None else None
                picked = [e for e in (edge, face) if e is not None]
            if picked:
                viewport.scene.select(picked)
                self._refresh_box(viewport)
                viewport.update()
                return
            viewport.flash_status(
                tr("Select (or click) the geometry to scale first"))

    def _grab(self, viewport, grip: _Grip, screen_xy) -> None:
        self._capture_grip(viewport, grip, screen_xy)
        self._machine.set("grab", screen_xy)

    def _capture_grip(self, viewport, grip, screen_xy):
        from core.image_plane import ImagePlane
        from core.mesh import Edge, Face

        sel = viewport.scene.selection
        self._groups = [g for g in sel if hasattr(g, "mesh")
                        and not isinstance(g, (Edge, Face))]
        self._images = [im for im in sel if isinstance(im, ImagePlane)]
        positions: list[QVector3D] = []
        for ent in sel:
            if isinstance(ent, Edge):
                positions.extend([ent.a, ent.b])
            elif isinstance(ent, Face):
                positions.extend(ent.vertices)
        seen: list[QVector3D] = []
        for p in positions:
            if not any((p - q).length() < 1e-9 for q in seen):
                seen.append(QVector3D(p))
        self._positions = seen
        mesh = viewport.scene.mesh
        self._verts = [v for v in (mesh.vertex_at(p) for p in seen)
                       if v is not None]
        self._grip = grip
        self._anchor = self._anchor_for(grip)
        self._factors = (1.0, 1.0, 1.0)
        self._capture_originals(viewport)
        viewport.update()

    def host_hover_grip(self, viewport, xy):
        self._refresh_box(viewport)
        hover = self._grip_under(viewport, *xy) if self._lo is not None else None
        if hover is not self._hover_grip:
            self._hover_grip = hover
            viewport.update()

    def host_scale_preview(self, viewport, factors):
        self._apply_preview(viewport, factors)
        viewport.update()

    def host_preview_factors(self, viewport, _):
        return self._factors

    def on_release(self, viewport):
        self._send(viewport, {"type": "release"})

    def host_toggle_center(self, viewport, say):
        self.about_center = not self.about_center
        self._reanchor(viewport)
        if say:
            viewport.flash_status(tr("About Center: on") if self.about_center
                                  else tr("About Center: off"))
        viewport.update()

    def host_toggle_uniform(self, viewport, _):
        self.uniform = not self.uniform
        viewport.flash_status(tr("Scale Uniformly: on") if self.uniform
                              else tr("Scale Uniformly: off"))
        viewport.update()

    def on_key(self, viewport, key, modifiers):
        name = {Qt.Key_Control: "Control", Qt.Key_Shift: "Shift"}.get(key)
        return bool(name and self._send(viewport, {"type": "key", "key": name})["consumed"])

    def on_key_release(self, viewport, key):
        if key != Qt.Key_Control:
            return False
        return self._send(viewport, {"type": "key_release", "key": "Control",
                                     "tapped": getattr(viewport, "ctrl_tapped", lambda: True)()})["consumed"]

    def _reanchor(self, viewport) -> None:
        """Ctrl mid-operation moves the anchor between centre and opposite
        side without losing the factors already dragged."""
        if self._grip is None:
            return
        current = self._factors
        self._apply_preview(viewport, (1.0, 1.0, 1.0))
        self._anchor = self._anchor_for(self._grip)
        self._apply_preview(viewport, current)
        viewport.update()

    def value_is_unitless(self) -> bool:
        """A bare number is a FACTOR, never a length (#176); one typed
        with a unit is the new size and still converts."""
        return True

    def on_value(self, viewport, value) -> bool:
        absolute = isinstance(value, tuple) and value and value[0] == "abs_len"
        if absolute:
            value = value[1]
        values = value if isinstance(value, tuple) else (value,)
        if not all(isinstance(v, float) for v in values):
            return False
        result = self._send(viewport, {"type": "absolute_value" if absolute else "value",
                                      "value": value})
        if result["redraw"]:
            viewport.update()
        return result["consumed"]

    def _typed_factors(self, value, absolute: bool, spec=None):
        values = value if isinstance(value, tuple) else (value,)
        if not all(isinstance(v, float) for v in values):
            return None
        return (spec or self._grip_spec()).typed(values, absolute)

    def host_cancel_box(self, viewport, _) -> None:
        self._revert_preview(viewport)
        self._grip = None
        self._anchor = None
        viewport.update()

    # ---- Live preview -------------------------------------------------------
    def _capture_originals(self, viewport) -> None:
        """Where everything that scales stood when the grip was taken. The
        live preview is computed from THESE on every move — never as a step
        from the previous frame. Chained steps drift in float32, and a drag
        that went down to 0.04 and back ended with some vertices a hair off
        their starting cell: the commit, which finds the vertices by their
        starting positions, missed them, scaled the rest and tore the
        faces (a user's video, 25-09: a rounded box shredded on release).
        Measured: 300 moves of that kind, 194 of 1600 vertices lost."""
        from ingetrazo_core import Matrix4 as QMatrix4x4
        orig_v: list = []
        orig_x: list = []
        for group in self._groups:
            if getattr(group, "xform", None) is not None:
                orig_x.append((group, QMatrix4x4(group.xform)))
            else:
                gmesh = group.mesh
                orig_v.extend((gmesh, v, QVector3D(v.position))
                              for v in gmesh.vertices)
        mesh = viewport.scene.mesh
        orig_v.extend((mesh, v, QVector3D(v.position)) for v in self._verts)
        self._orig_v = orig_v
        self._orig_x = orig_x
        self._orig_im = [(im, QVector3D(im.origin), QVector3D(im.u),
                          QVector3D(im.v)) for im in self._images]

    def _set_preview(self, viewport, factors: tuple) -> None:
        """Put everything at *factors* about the anchor, from the originals.
        At (1, 1, 1) it is an exact restore: the starting positions go back
        as they were stored, not through a matrix."""
        if getattr(self, "_orig_v", None) is None:
            return
        identity = all(abs(f - 1.0) < 1e-12 for f in factors)
        m = None if identity else scale_matrix(self._anchor, factors,
                                               self._axes)
        for mesh, v, p0 in self._orig_v:
            target = p0 if m is None else m.map(p0)
            mesh.move_vertex(v, target - v.position)
        for group, x0 in self._orig_x:
            group.xform = QMatrix4x4(x0) if m is None else m * x0
        for im, o0, u0, v0 in self._orig_im:
            if m is None:
                im.origin, im.u, im.v = (QVector3D(o0), QVector3D(u0),
                                         QVector3D(v0))
            else:
                im.origin, im.u, im.v = (m.map(o0), m.mapVector(u0),
                                         m.mapVector(v0))
        viewport.scene.version += 1

    def _apply_preview(self, viewport, target: tuple) -> None:
        if all(abs(t - f) < 1e-12 for t, f in zip(target, self._factors)):
            return
        self._set_preview(viewport, target)
        self._factors = target

    def _revert_preview(self, viewport) -> None:
        if any(abs(f - 1.0) > 1e-12 for f in self._factors):
            self._set_preview(viewport, (1.0, 1.0, 1.0))
            self._factors = (1.0, 1.0, 1.0)

    # ---- Commit -------------------------------------------------------------
    def _commit(self, viewport, factors: tuple) -> None:
        self._send(viewport, {"type": "scale_factors", "value": factors})

    def host_revert(self, viewport, _):
        self._revert_preview(viewport)

    def host_grip_spec(self, viewport, _):
        grip = self._grip
        return {"extents": self._extents(), "mask": [i in grip.mask for i in range(3)],
                "uniform": self.uniform or grip.kind(self._active_axes()) == "corner"}

    def host_scale_targets(self, viewport, _):
        scene = viewport.scene
        return {"anchor": self._anchor.toTuple(),
                "vertices": [str(v.key) for v in self._verts],
                "placements": [scene.occurrence_path(g) or g.uid for g in self._groups],
                "images": [scene.image_planes.index(im) for im in self._images],
                "axes": [a.toTuple() for a in self._axes] if self._axes else None}

    def host_end_grip(self, viewport, _):
        self._grip = None
        self._anchor = None
        self._factors = (1.0, 1.0, 1.0)
        self._orig_v = None                 # the operation is over
        self._box_version = -1              # geometry moved: rebuild the box
        self._refresh_box(viewport)
        viewport.update()

    def _reset(self) -> None:
        self._lo = self._hi = None
        self._grips = []
        self._grip = None
        self._hover_grip = None
        self._anchor = None
        self._factors = (1.0, 1.0, 1.0)
        self._groups = []
        self._positions = []
        self._verts = []
        self._images = []
        self._orig_v = None
        self._box_version = -1

    # ---- Feedback -----------------------------------------------------------
    def vcb_caption(self) -> str:
        """Caption the Measurements box by the axes in play."""
        grip = self._grip or self._hover_grip
        if grip is None:
            return "Scale"
        if self.uniform or grip.kind(self._active_axes()) == "corner":
            return "Scale"
        names = ", ".join(_AXIS_NAMES[i] for i in grip.mask)
        return f"{names} Scale"

    def value_label(self):
        """The live factor readout beside the box: one number
        for a uniform scale, one per axis otherwise."""
        if self._grip is None:
            return None
        if self.uniform or self._grip.kind(self._active_axes()) == "corner":
            text = f"{self._factors[self._grip.mask[0]]:.2f}"
        else:
            text = "; ".join(f"{self._factors[i]:.2f}"
                             for i in self._grip.mask)
        pos = (self._grip_pos(self._grip) if self._lo is not None
               else self._anchor)
        return (text, pos)

    # ---- What the viewport draws -------------------------------------------
    def scale_box_state(self):
        """Everything the overlay needs: the (live-scaled) box segments and
        each grip with its role — ``idle`` green, ``hover``/``active``/
        ``anchor`` red, as the official doc describes them."""
        if self._lo is None:
            return None
        lo, hi = self._lo, self._hi
        m = (scale_matrix(self._anchor, self._factors, self._axes)
             if self._grip is not None else None)

        def corner(tx, ty, tz):
            p = self._to_world(QVector3D(lo.x() + (hi.x() - lo.x()) * tx,
                                         lo.y() + (hi.y() - lo.y()) * ty,
                                         lo.z() + (hi.z() - lo.z()) * tz))
            return m.map(p) if m is not None else p

        segments = [(corner(*a), corner(*b)) for a, b in self._box_edges]
        grips = []
        for g in self._grips:
            p = self._grip_pos(g)
            if m is not None:
                p = m.map(p)
            role = "idle"
            if g is self._grip:
                role = "active"
            elif g is self._hover_grip and self._grip is None:
                role = "hover"
            grips.append((p, role))
        if self._grip is not None and self._anchor is not None:
            grips.append((QVector3D(self._anchor), "anchor"))
        return {"segments": segments, "grips": grips}
