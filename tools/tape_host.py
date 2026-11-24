# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Viewport picks, cursor and overlays for the Rust measuring tape."""
from tools.overlay_painter import draw_overlay
from PySide6.QtCore import Qt
from ingetrazo_core import Vec3 as QVector3D

from core.guide import Guide
from core.i18n import tr
from core.units import fmt_len, fmt_len_fine
from tools.base import Tool


class TapeHost(Tool):
    _source = None
    work_plane = None
    chain_first_point = None
    _MODES = (("line", "Guides: guide lines"), ("point", "Guides: guide points"),
              ("measure", "Guides: off — measure only"))

    def status_clause(self) -> str:
        """The usual wording, kept on screen: «Ctrl = Líneas guía del
        ciclo/Puntos guía/Medida». The active one is bracketed, because the
        cursor's + says "this leaves something" but not WHICH of the two
        guide modes is on."""
        names = {"line": tr("guide lines"), "point": tr("guide points"),
                 "measure": tr("measure")}
        parts = [f"[{v}]" if k == self._mode else v for k, v in names.items()]
        return "Ctrl = " + " / ".join(parts)

    @property
    def _guides(self) -> bool:
        """Whether this mode leaves anything behind."""
        return self._mode != "measure"

    @property
    def cursor_plus(self) -> bool:
        """The little + beside the cursor: this one will leave a
        guide — a line or a point."""
        return self._guides

    # ---- Lifecycle ----------------------------------------------------------
    def start_from_hover(self, viewport, pixel=None) -> bool:
        """Picking T over a line starts its guide without a first click."""
        from PySide6.QtCore import QPointF
        from views import input_state
        from views.document_interaction import _HoverEvent
        if self.start_point is not None or self._mode != "line":
            return False
        if pixel is None:
            pixel = viewport.mapFromGlobal(input_state.cursor_position(viewport))
        if not viewport.rect().contains(pixel):
            return False                 # toolbar/menu pickup outside the canvas
        ev = _HoverEvent(QPointF(pixel), Qt.NoModifier)
        ctx = viewport._build_ctx(ev)
        if ctx is None or (ctx.snap is not None and
                (ctx.snap.kind == "close" or self._machine.query("named", ctx.snap.kind))):
            return False                 # named points keep their point workflow
        edge = viewport.pick_edge_any(pixel.x(), pixel.y())
        guide = viewport.pick_guide(pixel.x(), pixel.y()) if edge is None else None
        axis = (viewport.pick_axis(pixel.x(), pixel.y())
                if edge is None and not getattr(guide, "is_line", False) else None)
        if edge is None and not getattr(guide, "is_line", False) and axis is None:
            return False
        # Use the normal click dispatch to retain face-plane capture,
        # snapping, value-buffer clearing and the operation's lock lifecycle.
        viewport._dispatch_tool_click(ev)
        if self._edge is None:
            self._send(viewport, {"type": "cancel"})
            return False
        self.hover_point = QVector3D(self.start_point)
        viewport.measurementChanged.emit(viewport._measurement_text())
        viewport.update()
        return True

    def host_clear_source(self, vp, _):
        self._source = self.work_plane = None

    def on_click(self, ctx):
        self._snap, self._click_point = ctx.snap, ctx.world
        try:
            super().on_click(ctx)
        finally:
            self._snap = self._click_point = None

    def on_value(self, vp, value):
        result = self._send(vp, {"type": "value", "value": value})
        if result["redraw"]:
            vp.update()
        return result["consumed"]

    def _toggle_mode(self, vp):
        return self._send(vp, {"type": "key_release", "key": "Control", "tapped": True})["consumed"]

    def host_tape_mode(self, vp, mode):
        self._during["mode"] = mode
        vp.flash_status(tr(dict(self._MODES)[mode]))
        for name in ("_apply_tool_cursor", "refresh_status_hint"):
            callback = getattr(vp, name, None)
            if callback:
                callback()
        vp.update()

    def on_key(self, vp, key, modifiers):
        return key == Qt.Key_Control and self._send(vp, {"type": "key", "key": "Control"})["consumed"]

    def on_key_release(self, vp, key):
        return key == Qt.Key_Control and self._send(vp, {
            "type": "key_release", "key": "Control",
            "tapped": getattr(vp, "ctrl_tapped", lambda: True)()})["consumed"]

    def host_tape_message(self, vp, args):
        label, distance = args
        captions = {"line": "Guide at {d}", "point": "Guide point at {d}",
                    "segment": "Guide point at {d}, with its segment", "measure": "Distance: {d}"}
        vp.flash_status(tr(captions[label]).format(d=fmt_len_fine(distance)),
                        4000 if label == "measure" else 3000)

    def guide_preview_lines(self):
        reading = self._machine.query("guide", None)
        return [Guide(*(QVector3D(*p) for p in reading)).segment()] if reading else []

    def value_label(self):
        reading = self._machine.query("label", None)
        return (fmt_len(reading[0]), QVector3D(*reading[1])) if reading else None

    overlay_kind = "tape"
    draw_overlay = draw_overlay
