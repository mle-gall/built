# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Viewport bindings for the Rust Move tool, including live deformation and grips."""
from __future__ import annotations

from tools.overlay_painter import draw_overlay
from PySide6.QtCore import Qt
from ingetrazo_core import Vec3 as QVector3D

from core.i18n import tr
from tools.base import ToolContext
from tools.rust_tool_base import RustTool, _mods, _point_property, _tuple
from core.units import fmt_len



from ingetrazo_core import (CopyWire, GripTurn, MoveGesture, TransformPose, moving_faces, rotation_handles as rotation_grips,
                            snap_exclusions, track_move_grips, transform_records, transform_targets as gather_targets)


def gather_images(ctx):
    return transform_records(ctx, "image_plane")


def gather_dimensions(ctx, groups, vertices):
    from core.move_ops import dimension_modes
    return dimension_modes(transform_records(ctx, "dimension"), groups, vertices)


class CopyPreview:
    def _gather_copy_entities(self, ctx):
        CopyWire.capture(self, ctx)

    def _fill_groups_ghost(self):
        self._base_segments.fill_groups()


#: How near (px) the cursor must come to a rotation grip to take it.
GRIP_PX = 9.0


class MoveHost(CopyPreview, RustTool):
    """Move (``rust/ingetrazo_core/src/tools/move_tool.rs`` decides: the
    grab, the Ctrl copy, typed distances and deltas, the array window).
    This class is the host: what a click grabs (it is picking), the live
    preview of the grabbed geometry, the rotation grips (#115, run by the
    Rotate tool) and the commit through ``core.move_ops``."""

    kind = ""
    _READ_DURING = ("grab", "hover_point", "copy", "window")
    # Drag on a camera-facing vertical plane (not the ground) so pulling the
    # mouse up raises geometry — what lets you lift a roof ridge straight up.
    prefers_vertical_drag = True
    accepts_array = True  # VCB "3x" / "/3" after a copy (arrays)

    def __init__(self) -> None:
        RustTool.__init__(self)
        self.chain_first_point = None  # silence close-snap
        self._ctx = None
        self._move_gesture = MoveGesture()
        self._clear_sets()
        # Rotation grips (issue #115): the group they sit on, the grips,
        # the one under the cursor, and — once one is taken — the Rotate
        # tool that runs the turn (its protractor, snaps, VCB and commit).
        self._grip_group = None
        self._grips: list = []
        self._hot_grip: int | None = None
        self._turn = GripTurn()

    _grip_rot = property(lambda self: self._turn.active)

    def _clear_sets(self) -> None:
        self._move_gesture.finish(self, self._state("window") is not None)

    # ---- The machine's state -----------------------------------------------
    start_point = _point_property("grab")
    grab = _point_property("grab")
    hover_point = _point_property("hover_point")

    @property
    def _copy(self) -> bool:
        return bool(self._state("copy"))

    @_copy.setter
    def _copy(self, on) -> None:
        self._machine.set("copy", bool(on))

    @property
    def cursor_plus(self) -> bool:
        """The little + beside the cursor while Ctrl has it making a COPY."""
        return self._copy

    def _after(self, viewport):
        if self.grab is None:
            self._clear_sets()
        update = getattr(viewport, "update", None)
        if callable(update):
            update()

    # ---- Lifecycle ----------------------------------------------------------
    def on_activate(self, viewport) -> None:
        self._turn.clear()
        self._clear_grips()
        self._send(viewport, {"type": "activate"})
        self._after(viewport)

    def on_deactivate(self, viewport) -> None:
        self._turn.cancel(viewport)
        self._clear_grips()
        self._send(viewport, {"type": "deactivate"})
        self._handles.clear()
        self._after(viewport)

    # ---- Keyboard -----------------------------------------------------------
    def on_key(self, viewport, key: int, modifiers) -> bool:
        if self._grip_rot is not None:
            return self._grip_rot.on_key(viewport, key, modifiers)
        # Ctrl toggles copy mode (move a copy, the original stays).
        if key == Qt.Key_Control:
            self._send(viewport, {"type": "key", "key": "Control", "mods": _mods(modifiers)})
            self._copy_changed(viewport)
            return True
        return False

    def _copy_changed(self, viewport) -> None:
        apply = getattr(viewport, "_apply_tool_cursor", None)
        if apply is not None:
            apply()                      # the + appears or disappears now
        viewport.update()

    def _toggle_copy(self, viewport, say: bool = True) -> None:
        """Ctrl pressed with the click (the Metal view's own check)."""
        self._send(viewport, {"type": "key", "key": "ToggleCopy" if say else "ToggleCopyQuiet"})
        self._copy_changed(viewport)

    def on_key_release(self, viewport, key: int) -> bool:
        if self._grip_rot is not None:
            return self._grip_rot.on_key_release(viewport, key)
        if key != Qt.Key_Control:
            return False
        tapped = getattr(viewport, "ctrl_tapped", None)
        reply = self._send(viewport, {"type": "key_release", "key": "Control",
                                      "tapped": not callable(tapped) or bool(tapped())})
        if reply["consumed"]:
            self._copy_changed(viewport)
        return reply["consumed"]

    # ---- Spatial input ------------------------------------------------------
    def on_click(self, ctx: ToolContext) -> None:
        viewport = ctx.viewport
        if self._turn.click(self, ctx):
            return
        self._ctx = ctx
        try:
            self._send(viewport, self._pointer("click", ctx))
        finally:
            self._ctx = None
        self._after(viewport)

    def on_hover(self, ctx: ToolContext) -> None:
        if self._grip_rot is not None:
            self._grip_rot.on_hover(ctx)
            return
        if self.start_point is None:
            self._track_grips(ctx)
        self._send(ctx.viewport, self._pointer("hover", ctx))
        ctx.viewport.update()

    def value_is_unitless(self) -> bool:
        """A rotation grip held (or just used) takes an ANGLE (#176);
        otherwise the value is a distance."""
        return self._turn.takes_angle and self.start_point is None

    def on_value(self, viewport, value) -> bool:
        turn = self._turn.value(self, viewport, value)
        if turn is not None:
            return turn
        consumed = self._send(viewport, {"type": "value", "value": value})["consumed"]
        self._after(viewport)
        return consumed

    def array_pending(self, viewport) -> bool:
        """Whether a copy was just made, so «3x» / «/3» would lay an array:
        the viewport then lets a bare «x» open the value box (#542)."""
        window = self._state("window")
        if window is None or self.start_point is not None:
            return False
        from tools.rust_tool_base import _Host
        return window["token"] == _Host(viewport).revision()

    @property
    def _last(self):
        return self._state("window")

    def close_array_window(self) -> None:
        """Esc right after a copy: «3x» / «/3» stop applying and a bare X
        is the Text shortcut again (#542)."""
        self._send(None, {"type": "key", "key": "CloseArray"})
        self._move_gesture.close_array()

    def on_array_value(self, viewport, count: int, mode: str,
                       step: float | None = None) -> bool:
        """Arrays, typed right after a Move-copy (``3x``, ``/3``, ``5x10m``,
        #111): the machine decides, the copy builder stamps."""
        done = self._send(viewport, {"type": "array", "count": int(count), "mode": mode,
                                     "step": None if step is None else float(step)})["consumed"]
        self._after(viewport)
        return done

    def on_cancel(self, viewport) -> None:
        if self._turn.cancel(viewport):
            self._clear_grips()
            viewport.update()
            return
        self._send(viewport, {"type": "cancel"})
        self._after(viewport)

    # ---- Visual preview -----------------------------------------------------
    def rubber_band_lines(self):
        if self._grip_rot is not None:
            return self._grip_rot.rubber_band_lines()
        return self._base_segments.move_lines(self.grab, self.hover_point, self._copy)

    def value_label(self):
        if self._grip_rot is not None:
            return self._grip_rot.value_label()
        grab, hover = self.grab, self.hover_point
        if grab is None or hover is None:
            return None
        delta = hover - grab
        mid = (grab + hover) * 0.5
        return (fmt_len(delta.length()), mid)

    # ---- Snap exclusion -----------------------------------------------------
    def snap_excluded(self):
        """What is in motion, for the snap engine to leave out of its
        candidates: ``(edge ids, group ids)`` or ``None``. The entities
        being moved are excluded from inference — otherwise the tool snaps
        to the very geometry it is dragging (issue #19, @pacaeiro). In copy
        mode nothing moves (the ghost is a wireframe), so nothing is left
        out."""
        if self._grip_rot is not None:
            return self._grip_rot.snap_excluded()
        if self.grab is None or self._copy:
            return None
        return snap_exclusions(self, False)

    def moving_face_ids(self) -> set:
        """The loose faces the drag bends — any face with a corner on a
        grabbed vertex. The viewport keeps the cursor from landing on
        them (it chased its own face and shook)."""
        if self.grab is None or self._copy:
            return set()
        return moving_faces(self)

    def moving_guide_ids(self) -> set:
        """The guides the drag carries (#627). They pass through the
        cursor all the way, so the viewport keeps them out of the snap
        candidates — the cursor would otherwise lock onto its own guide."""
        if self.grab is None or self._copy:
            return set()
        return {id(g) for g in self._moved_guides}

    # ---- Rotation grips (issue #115) ----------------------------------------
    @property
    def _drag(self):
        """Busy while a grip is held — the viewport's Esc asks this."""
        return self._grip_rot

    @property
    def wireframe_color(self):  # type: ignore[override]
        # While a grip turns the object, the protractor's axis colour.
        return (self._grip_rot.wireframe_color
                if self._grip_rot is not None else None)

    def drag_plane(self, viewport):
        """While a grip turns the object the cursor is read on the plane of
        the turn — the face the grip sits on."""
        if self._grip_rot is not None:
            return (QVector3D(self._grip_rot.start_point),
                    QVector3D(self._grip_rot._axis()))
        return None

    def _clear_grips(self) -> None:
        self._grip_group = None
        self._grips = []
        self._hot_grip = None

    def _track_grips(self, ctx):
        track_move_grips(self, ctx, GRIP_PX)

    def _grip_under(self, viewport, x, y):
        from ingetrazo_core import move_grip_hit
        return move_grip_hit(viewport, self._grips, (x, y), GRIP_PX) if self._grips else None

    overlay_kind = "move"
    draw_overlay = draw_overlay


    # ---- Internals ----------------------------------------------------------
    @staticmethod
    def _hovered_vertex(ctx):
        from ingetrazo_core import move_endpoint
        return move_endpoint(ctx)

    def _make_copy_builder(self, scene=None):
        """The copy command for a list of offsets, kept by the array window
        so ``3x`` / ``/3`` re-lay the copies (``core.move_ops.copy_builder``,
        the implementation the protocol's ``move_selection`` runs too)."""
        from core.move_ops import copy_builder
        return copy_builder(scene, self._groups, self._sel_faces, self._sel_edges,
                            self._moved_guides)

    def _shift(self, viewport, step: QVector3D) -> None:
        TransformPose(self, viewport.scene, "move", False).translate(viewport.scene, step)

    def _commit(self, viewport, delta) -> None:
        """Commit what is grabbed by ``delta`` (as a typed delta would)."""
        if self.grab is None:
            self._machine.set("grab", (0.0, 0.0, 0.0))
        d = delta.toTuple() if hasattr(delta, "toTuple") else tuple(delta)
        self._send(viewport, {"type": "value", "value": tuple(map(float, d))})
        self._after(viewport)

    @staticmethod
    def _tell_join(viewport, commands) -> None:
        """Say what became of a landing on other vertices: joined, or
        refused (and the move undone) because the join would break a
        face."""
        flash = getattr(viewport, "flash_status", None)
        if flash is None:
            return
        refused = getattr(viewport.history, "last_rejected", None)
        if refused:
            flash(refused, 8000)
        elif any(getattr(c, "welded", False) for c in commands):
            flash(tr("Vertices joined"))

    def _reset(self) -> None:
        self._machine.set("reset", True)
        self._clear_sets()
