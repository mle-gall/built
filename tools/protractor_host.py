# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Protractor tool (Shift+H) and the shared protractor mechanics.

:class:`ProtractorBase` holds everything the protractor cursor does —
Rotate (Q) shows the same instrument, so both tools share it:

- Before the first click the disc follows the cursor, aligned to the face
  underneath (empty ground measures in plan) and coloured by the axis it
  rotates about — red/green/blue on an axis plane, dark otherwise. Arrow keys
  lock the plane to an axis (Right=red, Left=green, Up=blue; same arrow again
  releases); holding Shift freezes the current plane. The plane fixes once
  the vertex/centre is placed.
- The disc keeps a fixed SCREEN size, with tick marks every 15° (long at
  90°), zero aligned to the base arm once set.
- The distance rule: near the disc the cursor snaps to the 15° ticks;
  farther out the angle is free at 0.1° precision.
- The Measurements box accepts degrees (``34.1``) or a slope as rise:run
  (``3:12``, ``1:6``) — ``accepts_angle_ratio`` delivers it as degrees.
- CLICK-DRAG from the vertex tilts the instrument: the drag sets the
  protractor's axis (the normal of its plane) along the dragged direction,
  off the orthogonal planes — the same gesture on both Protractor and
  Rotate (issue #10, @pacaeiro). A plain click keeps the inferred plane.

:class:`tools.rust_tools.ProtractorTool` creates ANGLED guide lines with it
(the classic angled-guide workflow): vertex → base arm →
sweep & click = an infinite dashed guide through the vertex. The tool then
resets, but the angle stays "hot": typing a value re-aims the guide just
created until the next click or tool change.

Guides are scaffolding, not geometry: they feed the snap engine so Line /
Rectangle can lock onto the angled direction, and are deleted with Select /
Eraser / Edit ▸ Delete Guides.
"""
from __future__ import annotations

import math

from PySide6.QtCore import Qt
from ingetrazo_core import Vec3 as QVector3D

from core.guide import Guide
from ingetrazo_core import ToolMachine
from core.snap import COLOR_AXIS_X, COLOR_AXIS_Y, COLOR_AXIS_Z
from core.axes import plane_axes  # drawing axes (#44)
from tools.base import Tool, ToolContext

# The disc keeps a fixed SCREEN size; ticks every 15 degrees.
DISC_PX = 60.0
TICK_DEG = 15.0
# The drawing axes (core.axes): the open group's own inside it (#44).
from core.axes import AXES as _AXES  # noqa: E402
_AXIS_RGBA = {"x": (*COLOR_AXIS_X, 1.0), "y": (*COLOR_AXIS_Y, 1.0),
              "z": (*COLOR_AXIS_Z, 1.0)}
_OFF_AXIS_RGBA = (0.24, 0.27, 0.32, 1.0)


class ProtractorBase(Tool):
    """Shared protractor state + behaviour (see module docstring)."""

    def value_is_unitless(self) -> bool:
        """The typed value is an ANGLE: the document's length unit must
        not scale it («45» was 0.045° in a millimetre model, #176)."""
        return True

    #: The Line tool's axis magnet for the arms (@pacaeiro, issue #41): a
    #: base arm within 3° of an axis that lies in the disc's plane lands
    #: on it. Only the world half — the screen detector would hand back a
    #: point on an axis OUTSIDE the disc's plane, which an arm cannot be.
    magnetic_axis_deg = 3.0
    #: The arms are directions from the centre, not lines being drawn: the
    #: snap engine leaves out 'through point', 'extension' and 'from point'
    #: for them, so the axis magnet is not outvoted (issue #140).
    radial_arm = True

    def __init__(self) -> None:
        self.start_point: QVector3D | None = None   # the protractor centre
        self.ref_point: QVector3D | None = None     # base (0°) direction
        self.hover_point: QVector3D | None = None
        self.work_plane: tuple[QVector3D, QVector3D] | None = None
        #: The plane the centre was placed on, kept so an arrow-key lock set
        #: after the first click can be released back to it.
        self._centre_plane: tuple[QVector3D, QVector3D] | None = None
        self._axis_pick: str | None = None          # arrow-key plane lock
        self._shift_normal: QVector3D | None = None  # Shift-frozen plane
        self._custom_axis: QVector3D | None = None  # drag-defined axis
        self._axis_drag_armed = False               # centre press → release watches
        #: While that drag is past the threshold, the axis it WOULD set —
        #: so the disc tilts live instead of the gesture happening in the
        #: dark (issue #25, @pacaeiro). One source of truth: the release
        #: commits exactly the axis the preview showed.
        self._axis_drag_live: QVector3D | None = None
        self._disc_r = 1.0                          # world radius of the disc
        self._snap_ticks = False                    # cursor near the disc?
        #: The cursor is held by an inference (an endpoint, an axis, an
        #: intersection...): the arm must reach THAT point exactly.
        self._exact_snap = False

    # ---- Click-drag axis ----------------------------------------------------
    #: How far the cursor must travel from the vertex for the press to read
    #: as a DRAG that tilts the instrument, rather than a plain click.
    _AXIS_DRAG_PX = 8.0

    def _track_axis_drag(self, viewport) -> None:
        """While the centre is held, decide on every move whether the drag
        has gone far enough to tilt the instrument — and remember the axis
        it would set, so the disc shows it as the hand moves. Before this
        the gesture gave no sign at all until the button came up
        (issue #25, @pacaeiro: «could have live preview, indicating that it
        is reacting to the mouse movement»)."""
        if not self._axis_drag_armed:
            self._axis_drag_live = None
            return
        if self.hover_point is None or self.start_point is None:
            self._axis_drag_live = None
            return
        w2p = getattr(viewport, "_world_to_pixel", None)
        if w2p is not None:
            p0, p1 = w2p(self.start_point), w2p(self.hover_point)
            if p0 is None or p1 is None or math.hypot(
                    p1[0] - p0[0], p1[1] - p0[1]) <= self._AXIS_DRAG_PX:
                self._axis_drag_live = None
                return
        d = self.hover_point - self.start_point
        self._axis_drag_live = (d.normalized() if d.length() > 1e-9
                                else None)

    def _release_axis_drag(self, viewport) -> bool:
        """A real DRAG from the centre fixes the instrument's axis along it
        (the fold gesture on Rotate, and the Protractor's way off the
        orthogonal planes); a plain click keeps the inferred plane. Returns
        True when an axis was set."""
        if not self._axis_drag_armed:
            return False
        self._track_axis_drag(viewport)      # a release with no move in between
        self._axis_drag_armed = False
        axis = self._axis_drag_live
        self._axis_drag_live = None
        if axis is None:                     # a plain click keeps the plane
            return False
        self._custom_axis = axis
        viewport.update()
        return True

    # ---- Keyboard -----------------------------------------------------------
    def on_key(self, viewport, key: int, modifiers) -> bool:
        # Arrow keys lock the protractor plane to an axis: Right =
        # red, Left = green, Up = blue; the same arrow again releases it.
        picks = {Qt.Key_Right: "x", Qt.Key_Left: "y", Qt.Key_Up: "z"}
        axis = picks.get(key)
        if axis is None:
            return False
        self._axis_pick = None if self._axis_pick == axis else axis
        if self._axis_pick is not None:
            # The lock re-anchors the work plane so the cursor — and the axis
            # magnet — follow it. Before the centre the plane rides the
            # cursor; after the centre it is pinned to the centre.
            anchor = (self.start_point if self.start_point is not None
                      else (self.hover_point if self.hover_point is not None
                            else QVector3D(0, 0, 0)))
            self.work_plane = (QVector3D(anchor),
                               QVector3D(_AXES[self._axis_pick]))
        elif self.start_point is not None and self._centre_plane is not None:
            # Released after the centre: back to the plane it was placed on.
            self.work_plane = self._centre_plane
        viewport.update()
        return True

    # ---- Plane / colour -----------------------------------------------------
    @property
    def wireframe_color(self):  # type: ignore[override]
        # The disc is coloured by its rotation axis: red/green/blue
        # on an axis plane, dark on an arbitrary face plane.
        n = self._axis()
        for axis, v in _AXES.items():
            if abs(QVector3D.dotProduct(n, v)) > 0.999:
                return _AXIS_RGBA[axis]
        return _OFF_AXIS_RGBA

    def _axis(self) -> QVector3D:
        if self._custom_axis is not None:
            return self._custom_axis
        if self._shift_normal is not None:
            return self._shift_normal
        if self._axis_pick is not None:
            return QVector3D(_AXES[self._axis_pick])
        if self.work_plane is not None:
            return self.work_plane[1].normalized()
        return QVector3D(0.0, 0.0, 1.0)

    def _infer_plane(self, ctx: ToolContext) -> None:
        """Plane inference, active until the vertex is placed: the
        disc aligns to the face under the cursor (Shift freezes it, arrows
        override it); empty ground measures in plan."""
        shift = bool(ctx.modifiers & Qt.ShiftModifier)
        if not shift:
            self._shift_normal = None
        if self.start_point is not None:   # vertex placed: plane is fixed
            return
        if shift:
            if self._shift_normal is None:
                self._shift_normal = QVector3D(self._axis())
            return
        if self._axis_pick is not None:
            self.work_plane = (ctx.world, QVector3D(_AXES[self._axis_pick]))
            return
        pick = getattr(ctx.viewport, "pick_face_any", None)
        face = None
        if pick is not None:
            # Only its normal is read: a big lazy mesh stays unbuilt (#1092).
            from views.array_bake import reading_only
            with reading_only():
                face, _group = pick(ctx.screen.x(), ctx.screen.y())
        if face is not None:
            self.work_plane = (QVector3D(ctx.world), face.normal().normalized())
        else:
            self.work_plane = None         # ground: measure in plan

    # ---- Screen metrics / snapping ------------------------------------------
    def _update_screen_metrics(self, ctx: ToolContext) -> None:
        """Fixed screen-size disc + the distance rule: near the disc
        the cursor snaps to the 15° ticks, farther out it measures free at
        0.1° precision."""
        w2p = getattr(ctx.viewport, "_world_to_pixel", None)
        # NOT ``start or hover``: a zero QVector3D (the origin!) is falsy.
        centre = (self.start_point if self.start_point is not None
                  else self.hover_point)
        if w2p is None or centre is None:
            return
        u, v = plane_axes(self._axis())
        p0 = w2p(centre)
        p1 = w2p(centre + u)
        p2 = w2p(centre + v)
        if p0 is None or p1 is None or p2 is None:
            return
        # The larger of the two in-plane scales: along an axis that points
        # into the screen, one unit is a few pixels and the radius exploded.
        px_per_unit = max(math.hypot(p1[0] - p0[0], p1[1] - p0[1]),
                          math.hypot(p2[0] - p0[0], p2[1] - p0[1]))
        if px_per_unit > 1e-6:
            self._disc_r = DISC_PX / px_per_unit
        if self.start_point is not None:
            dist = math.hypot(ctx.screen.x() - p0[0], ctx.screen.y() - p0[1])
            self._snap_ticks = dist <= DISC_PX * 1.25
        else:
            self._snap_ticks = False

    # ---- Angles -------------------------------------------------------------
    def _basis(self):
        return [p.toTuple() for p in plane_axes(self._axis())]

    def _angle_to(self, point):
        arm = (self.ref_point - self.start_point).toTuple()
        other = (point - self.start_point).toTuple()
        return ToolMachine.protractor_angle(self._basis(), arm, other)

    def _display_deg(self, point):
        angle = self._angle_to(point)
        return None if angle is None else ToolMachine.protractor_display(angle, self._snap_ticks)

    #: Snap kinds that are NOT a precise target: the free cursor and a
    #: point merely on a face. Everything else (endpoint, midpoint,
    #: intersection, on-edge, axis, guide...) is a point the user aimed at.
    _FREE_SNAPS = frozenset({"none", "on_face"})

    def _note_snap(self, ctx) -> None:
        """Remember whether the cursor is held by an inference."""
        snap = getattr(ctx, "snap", None)
        kind = getattr(snap, "kind", "none") or "none"
        self._exact_snap = kind not in self._FREE_SNAPS

    def _commit_deg(self, point: QVector3D) -> float | None:
        """The angle to APPLY: exact when the cursor sits on an inferred
        point, so the arm lands on it; otherwise the displayed
        value -- the 15° tick near the disc, 0.1° farther out.

        Issue #163: the rotation always applied the 0.1°-rounded angle, so
        a panel swung to an endpoint missed it by ~0.7 mm two metres out
        (70.2789° applied as 70.3°). The label still reads 0.1°."""
        if self._exact_snap and not self._snap_ticks:
            return self._angle_to(point)
        return self._display_deg(point)

    def _direction_at(self, deg):
        arm = (self.ref_point - self.start_point).toTuple()
        return QVector3D(*ToolMachine.protractor_direction(self._basis(), arm, deg))

    # ---- Disc rendering -----------------------------------------------------
    def _protractor_disc(self, centre: QVector3D, axis=None):
        """The fixed-screen-size disc with tick marks every 15° (long at 90°),
        rotated so its zero sits on the base arm once that is set. ``axis``
        borrows a different one — what the live tilt preview draws."""
        u, v = plane_axes(self._axis() if axis is None else axis)
        r = self._disc_r
        base = 0.0
        if self.start_point is not None and self.ref_point is not None:
            a = self.ref_point - self.start_point
            base = math.atan2(QVector3D.dotProduct(a, v),
                              QVector3D.dotProduct(a, u))

        def rim(t: float, k: float = 1.0):
            return centre + (u * math.cos(t) + v * math.sin(t)) * (r * k)

        n = 48
        pts = [rim(base + 2 * math.pi * k / n) for k in range(n)]
        segments = [(pts[k], pts[(k + 1) % n]) for k in range(n)]
        for k in range(int(360 / TICK_DEG)):
            t = base + math.radians(k * TICK_DEG)
            inner = 0.75 if k * TICK_DEG % 90 == 0 else 0.86
            segments.append((rim(t, inner), rim(t)))
        return segments

    def _reset_protractor(self) -> None:
        self.start_point = None
        self.ref_point = None
        self.work_plane = None
        self._centre_plane = None
        self._shift_normal = None
        self._custom_axis = None
        self._axis_drag_live = None
        self._axis_drag_armed = False
        self._snap_ticks = False
        self._exact_snap = False


class ProtractorHost(ProtractorBase):
    accepts_angle_ratio = True
    _held_guide = None

    @property
    def cursor_plus(self) -> bool:
        return self._guides

    def status_clause(self) -> str:
        """Kept on screen while the tool is active. Two
        modes here: the Protractor cannot drop a guide POINT."""
        from core.i18n import tr as _tr
        guia, medir = _tr("guide"), _tr("measure")
        return ("Ctrl = " + (f"[{guia}] / {medir}" if self._guides
                             else f"{guia} / [{medir}]"))

    def on_activate(self, viewport):
        self._send(viewport, {"type": "activate"})
        self._axis_pick = self._held_guide = None

    def on_deactivate(self, viewport):
        self._send(viewport, {"type": "deactivate"})
        self._axis_pick = self._held_guide = self.hover_point = None

    def on_cancel(self, viewport):
        self._send(viewport, {"type": "cancel"})
        self._axis_pick = self._held_guide = None
        viewport.update()

    def _send_basis(self, viewport):
        self._send(viewport, {"type": "protractor_frame", "basis": self._basis(),
                              "near": self._snap_ticks, "exact": self._exact_snap})

    def on_click(self, ctx):
        if self.ref_point is not None:
            self._note_snap(ctx)
            self._send_basis(ctx.viewport)
        self._held_guide = None
        self._send(ctx.viewport, self._pointer("click", ctx))
        ctx.viewport.update()

    def on_hover(self, ctx):
        self.hover_point = ctx.world
        self._track_axis_drag(ctx.viewport)
        self._infer_plane(ctx)
        self._update_screen_metrics(ctx)
        self._note_snap(ctx)
        self._send(ctx.viewport, self._pointer("hover", ctx))
        ctx.viewport.update()

    def on_release(self, viewport) -> None:
        # Click-drag from the vertex: the protractor's axis follows the drag
        # (issue #10) — the guide then lies in the tilted plane.
        if self._release_axis_drag(viewport):
            from core.i18n import tr
            viewport.flash_status(tr("Protractor axis set along the drag"))

    def on_value(self, viewport, value):
        if not isinstance(value, (int, float)):
            return False
        self._send_basis(viewport)
        self._machine.set("hover_point", self.hover_point)
        result = self._send(viewport, {"type": "value", "value": value})
        if result["redraw"]:
            viewport.update()
        return result["consumed"]

    def _toggle_guides(self, viewport, say=True):
        self._send(viewport, {"type": "key", "key": "ToggleGuides" if say else "ToggleGuidesQuiet"})

    def on_key(self, viewport, key, modifiers):
        if key == Qt.Key_Control:
            return self._send(viewport, {"type": "key", "key": "Control"})["consumed"]
        return super().on_key(viewport, key, modifiers)

    def on_key_release(self, viewport, key):
        if key != Qt.Key_Control:
            return False
        return self._send(viewport, {"type": "key_release", "key": "Control",
                                     "tapped": getattr(viewport, "ctrl_tapped", lambda: True)()})["consumed"]

    def host_reset_disc(self, viewport, _):
        self._reset_protractor()

    def host_guide_toggle(self, viewport, args):
        from core.i18n import tr
        guides, say = args
        self._during["guides"] = guides
        if say:
            viewport.flash_status(tr("Create guides: on") if guides else tr("Create guides: off — measure only"))
        for name in ("_apply_tool_cursor", "refresh_status_hint"):
            callback = getattr(viewport, name, None)
            if callback:
                callback()
        viewport.update()

    def host_disc_centre(self, viewport, point):
        self._centre_plane = (QVector3D(*point), QVector3D(self._axis()))
        self._axis_drag_armed = True

    def host_remember_guide(self, viewport, _):
        self._held_guide = viewport.scene.guides[-1]

    def host_live_guide_index(self, viewport, _):
        return next((i for i, guide in enumerate(viewport.scene.guides) if guide is self._held_guide), None)

    def host_measured_angle(self, viewport, degrees):
        from core.i18n import tr
        from core.units import fmt_angle_deg
        viewport.flash_status(tr("Angle: {a}").format(a=fmt_angle_deg(degrees, signed=True)), 4000)

    def rubber_band_lines(self):
        centre = (self.start_point if self.start_point is not None
                  else self.hover_point)
        if centre is None:
            return []
        # Dragging from the vertex to tilt the instrument: draw the disc it
        # WOULD land on, so the hand sees the gesture take (issue #25). The
        # committed axis is untouched — the release decides, from the very
        # same ``_axis_drag_live`` this draws.
        tilt = self._axis_drag_live if self._axis_drag_armed else None
        segments = list(self._protractor_disc(centre, tilt))
        if self.start_point is None or self.hover_point is None:
            return segments
        segments.append((self.start_point, self.hover_point))
        if self.ref_point is not None:
            segments.append((self.start_point, self.ref_point))
        return segments

    def guide_preview_lines(self):
        """The angled guide the next click leaves, through the vertex."""
        if (not self._guides or self.start_point is None
                or self.ref_point is None or self.hover_point is None):
            return []
        deg = self._commit_deg(self.hover_point)
        if deg is None:
            return []
        return [Guide(self.start_point, self._direction_at(deg)).segment()]

    def value_label(self):
        if self.ref_point is None or self.hover_point is None:
            return None
        deg = self._display_deg(self.hover_point)
        if deg is None:
            return None
        from core.units import fmt_angle_deg
        return (fmt_angle_deg(deg, signed=True), self.hover_point)

    def vcb_caption(self) -> str:
        return "Angle"
