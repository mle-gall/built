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

from tools.overlay_painter import draw_overlay
from PySide6.QtCore import Qt
from ingetrazo_core import Vec3 as QVector3D

from ingetrazo_core import ToolMachine
from core.group import Group
from core.mesh import Edge, Face
from core.axes import AXES as _AXES
from core.i18n import tr
from tools.base import Tool, ctrl_clause



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
    overlay_kind = "flip"
    draw_overlay = draw_overlay


    # ---- Internals ----------------------------------------------------------
    def _bounds(self, viewport):
        from tools.rust_tool_base import selection_bounds
        return selection_bounds(viewport.scene).limits()

    def _plane_quad(self, axis, centre, half, margin):
        return [QVector3D(*p) for p in ToolMachine.flip_quad(axis, centre.toTuple(), half.toTuple(), margin)]

    def _plane_under_cursor(self, viewport, sx: float, sy: float):
        from tools.rust_tool_base import selection_bounds
        box = selection_bounds(viewport.scene)
        if box.limits()[0] is not None and hasattr(viewport, "_world_to_pixel"):
            return box.flip_pick(viewport, (sx, sy))
