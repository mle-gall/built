# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Flip tool — Flip (mirror).

Flow: with a selection, three semi-transparent planes appear over it —
red, green and blue, one per axis. Hovering highlights a plane; ONE CLICK
flips the selection about it. The arrow keys pick a plane (Right = red,
Left = green, Up = blue); tapping Ctrl toggles COPY mode, which leaves the
original and creates the flipped duplicate.

Deferred (documented): dragging a plane to reposition it, Alt
parent-context axes, and the magenta custom plane from hovering a face.
"""
from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtGui import QColor, QPen, QPolygonF
from ingetrazo_core import Vec3 as QVector3D
from PySide6.QtCore import QPointF

from ingetrazo_core import ToolMachine
from core.group import Group
from core.mesh import Edge, Face
from core.axes import AXES as _AXES
from core.i18n import tr
from tools.base import Tool, ctrl_clause

_PLANE_RGBA = {"x": (216, 56, 68), "y": (40, 158, 90), "z": (52, 102, 198)}


class FlipHost(Tool):
    def status_clause(self):
        return ctrl_clause(self.cursor_plus, "copy", "original")

    def host_empty_flip(self, vp, _):
        vp.flash_status(tr("Select the geometry to flip first"))

    def host_flip_selected(self, vp, _):
        return any(isinstance(e, (Group, Face, Edge)) for e in vp.scene.selection)

    def host_flip_targets(self, vp, axis):
        lo, hi = self._bounds(vp)
        sel = vp.scene.selection
        if not self.host_flip_selected(vp, None):
            return None
        centre = (lo + hi) * .5 if lo is not None else QVector3D()
        return {"center": centre, "axis": _AXES[axis],
                "placements": [g.uid for g in sel if isinstance(g, Group)],
                "faces": [str(f.key) for f in sel if isinstance(f, Face)],
                "edges": [str(e.key) for e in sel if isinstance(e, Edge)]}

    def host_flip_plane_at(self, vp, px):
        return self._plane_under_cursor(vp, *px)

    def host_flip_copy(self, vp, args):
        active, say = args
        self._during["copy"] = active
        if say:
            vp.flash_status(tr("Flip a copy: on") if active else tr("Flip a copy: off"))
        self.host_flip_cursor(vp, None)
        self.refresh_hint(vp)
        vp.update()

    def host_flip_cursor(self, vp, active):
        if active is not None:
            self._during["copy"] = active
        callback = getattr(vp, "_apply_tool_cursor", None)
        if callback:
            callback()

    def on_key_release(self, vp, key):
        return key == Qt.Key_Control and self._send(vp, {
            "type": "key_release", "key": "Control",
            "tapped": getattr(vp, "ctrl_tapped", lambda: True)()})["consumed"]

    def on_key(self, vp, key, modifiers):
        name = {Qt.Key_Control: "Control", Qt.Key_Right: "Right", Qt.Key_Left: "Left",
                Qt.Key_Up: "Up"}.get(key)
        if name is None:
            return False
        result = self._send(vp, {"type": "key", "key": name})
        vp.update()
        return result["consumed"]

    def flip(self, vp, axis, copy=False):
        result = self._send(vp, {"type": "flip_along", "axis": axis, "copy": copy})
        if result["committed"]:
            vp.update()
        return result["committed"]

    # ---- Overlay ------------------------------------------------------------
    def draw_overlay(self, viewport, painter) -> None:
        """The three semi-transparent axis planes over the selection; the
        hovered / arrow-locked one highlights."""
        lo, hi = self._bounds(viewport)
        if lo is None:
            return
        centre = (lo + hi) * 0.5
        half = (hi - lo) * 0.5
        margin = 1.15
        for axis, n in _AXES.items():
            quad = self._plane_quad(axis, centre, half, margin)
            pts = [viewport._world_to_pixel(q) for q in quad]
            if any(p is None for p in pts):
                continue
            poly = QPolygonF([QPointF(*p) for p in pts])
            r, g, b = _PLANE_RGBA[axis]
            active = axis == (self._lock_axis or self._hover_axis)
            painter.setPen(QPen(QColor(r, g, b), 2.2 if active else 1.2))
            painter.setBrush(QColor(r, g, b, 90 if active else 36))
            painter.drawPolygon(poly)
        painter.setBrush(Qt.NoBrush)

    # ---- Internals ----------------------------------------------------------
    def _bounds(self, viewport):
        import numpy as np
        from core.group import placement_points
        sel = viewport.scene.selection
        arrays: list = []
        pts: list = []
        for ent in sel:
            if isinstance(ent, Group):
                # Its nested placements too: a group made of groups keeps
                # its geometry there and its own mesh empty, and the box
                # came out empty — no planes, nothing to click (#380).
                # Kept as arrays: this runs on every hover.
                arr = placement_points(ent)
                if len(arr):
                    arrays.append(arr)
            elif isinstance(ent, Face):
                pts.extend((v.x(), v.y(), v.z()) for v in ent.vertices)
            elif isinstance(ent, Edge):
                pts.extend((p.x(), p.y(), p.z()) for p in (ent.a, ent.b))
        if pts:
            arrays.append(np.asarray(pts, dtype=np.float64))
        if not arrays:
            return None, None
        allp = np.concatenate(arrays)
        lo, hi = allp.min(axis=0), allp.max(axis=0)
        return QVector3D(*lo.tolist()), QVector3D(*hi.tolist())

    def _plane_quad(self, axis, centre, half, margin):
        return [QVector3D(*p) for p in ToolMachine.flip_quad(axis, centre.toTuple(), half.toTuple(), margin)]

    def _plane_under_cursor(self, viewport, sx: float, sy: float):
        """Which of the three planes the cursor is inside (projected); ties
        break toward the plane whose centre is nearest on screen."""
        lo, hi = self._bounds(viewport)
        if lo is None:
            return None
        centre = (lo + hi) * 0.5
        half = (hi - lo) * 0.5
        w2p = getattr(viewport, "_world_to_pixel", None)
        if w2p is None:
            return None
        best = None
        best_d = float("inf")
        for axis in _AXES:
            quad = self._plane_quad(axis, centre, half, 1.15)
            pts = [w2p(q) for q in quad]
            if any(p is None for p in pts):
                continue
            poly = QPolygonF([QPointF(*p) for p in pts])
            if not poly.containsPoint(QPointF(sx, sy), Qt.OddEvenFill):
                continue
            cpx = w2p(centre)
            mid = (sum(p[0] for p in pts) / 4.0, sum(p[1] for p in pts) / 4.0)
            d = (mid[0] - sx) ** 2 + (mid[1] - sy) ** 2
            if cpx is not None and d < best_d:
                best_d = d
                best = axis
        return best
