# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Viewport bindings for the Rust Move tool, including live deformation and grips."""
from __future__ import annotations

from PySide6.QtCore import Qt
from ingetrazo_core import Vec3 as QVector3D

from core.group import Group
from core.i18n import tr
from core.guide import Guide
from core.history import CompoundCommand
from core.textlabel import TextLabel
from tools.base import ToolContext
from tools.rust_tool_base import RustTool, _mods, _point_property, _tuple
from core.units import fmt_len



from ingetrazo_core import CopyWire, TransformPose, transform_records, transform_targets as gather_targets


def gather_images(ctx):
    return transform_records(ctx, "image_plane")


def gather_dimensions(ctx, groups, vertices):
    from core.move_ops import dimension_modes
    return dimension_modes(transform_records(ctx, "dimension"), groups, vertices)


class CopyPreview:
    def _gather_copy_entities(self, ctx):
        self._groups_ghost_due = True
        self._sel_faces, self._sel_edges, self._base_segments = CopyWire.gather(
            ctx, self._moved_guides, not self._groups and not self._moved_guides)

    def _fill_groups_ghost(self):
        # Imported components may be huge. Only read placement edges for a copy.
        if self._groups_ghost_due:
            self._groups_ghost_due = False
            for group in self._groups:
                self._base_segments.add_group(group)


#: How near (px) the cursor must come to a rotation grip to take it.
GRIP_PX = 9.0
#: Where the four grips of a face sit: this fraction of the face's size
#: from its centre, along each of its two directions.
def rotation_grips(obb, eye):
    from ingetrazo_core import move_rotation_grips
    if obb is None:
        return []
    frame, lo, hi = obb
    grips = move_rotation_grips([_tuple(v) for v in frame], _tuple(lo), _tuple(hi), eye.toTuple())
    return [tuple(QVector3D(*v) for v in grip) for grip in grips]


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
        self._build = None             # the copy builder of the array window
        self._clear_sets()
        # Rotation grips (issue #115): the group they sit on, the grips,
        # the one under the cursor, and — once one is taken — the Rotate
        # tool that runs the turn (its protractor, snaps, VCB and commit).
        self._grip_group = None
        self._grips: list = []
        self._hot_grip: int | None = None
        self._grip_rot = None
        self._grip_rot_done = None             # its hot retype after the commit

    def _clear_sets(self) -> None:
        self._pose = None
        self._positions: list[QVector3D] = []  # unique positions to translate
        self._verts: list = []                 # resolved vertex objects (identity)
        self._groups: list[Group] = []         # whole groups being moved
        self._splanes: list = []               # section planes being moved
        self._images: list = []                # reference images being moved
        self._labels: list[TextLabel] = []     # leader texts whose label moves
        self._moved_guides: list[Guide] = []   # construction guides being moved
        self._dims: dict = {}                  # dimension → "line" | "rigid"
        self._preview_delta = QVector3D(0.0, 0.0, 0.0)  # currently applied live
        self._sel_faces: list = []             # loose geometry copy mode duplicates
        self._sel_edges: list = []
        self._base_segments: list = []         # wireframe for the copy preview
        self._groups_ghost_due = False
        self._vp_preview = False

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

    # ---- Host verbs the machine calls (``ToolHost::call``) -----------------
    def host_grab(self, viewport, world):
        return self._grab_under(self._ctx, QVector3D(*world))

    def host_preview(self, viewport, delta):
        self._apply_preview(viewport, QVector3D(*delta))

    def host_park(self, viewport, _args):
        """Copy on: the original stops following the cursor."""
        self._revert_preview(viewport)
        park = getattr(viewport, "set_groups_preview_offset", None)
        if self._vp_preview and park is not None:
            park(QVector3D(0.0, 0.0, 0.0))

    def host_revert(self, viewport, _args):
        self._end_freeze(viewport)
        self._revert_preview(viewport)

    def host_move(self, viewport, delta):
        from core.move_ops import move_commands
        commands = move_commands(QVector3D(*delta), groups=self._groups, positions=self._positions,
                                 labels=self._labels, guides=self._moved_guides, dims=self._dims,
                                 splanes=self._splanes, images=self._images)
        if len(commands) == 1:
            viewport.history.execute(commands[0])
        elif commands:
            viewport.history.execute(CompoundCommand(commands))
        self._tell_join(viewport, commands)

    def host_copy(self, viewport, args):
        """Stamp copies at ``deltas``; ``fresh`` makes the builder from what
        is grabbed now, else the array window re-uses the last one.
        Answers the history token when the copy is the top of the stack."""
        if args["fresh"] or self._build is None:
            self._build = self._make_copy_builder(viewport.scene)
        cmd = self._build([QVector3D(*d) for d in args["deltas"]])
        if cmd is None:
            return None
        viewport.history.execute(cmd)
        stack = getattr(viewport.history, "undo_stack", None)
        if not stack or stack[-1] is not cmd:
            return None
        from tools.rust_tool_base import _Host
        return _Host(viewport).revision()

    # ---- Lifecycle ----------------------------------------------------------
    def on_activate(self, viewport) -> None:
        self._grip_rot = self._grip_rot_done = None
        self._clear_grips()
        self._send(viewport, {"type": "activate"})
        self._after(viewport)

    def on_deactivate(self, viewport) -> None:
        if self._grip_rot is not None:
            self._grip_rot.on_cancel(viewport)
        # A helper left here kept the next Move a Rotate (#480).
        self._grip_rot = self._grip_rot_done = None
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
        self._grip_rot_done = None           # a click ends the angle retype
        if self._grip_rot is not None:
            rot = self._grip_rot
            rot.on_click(ctx)                # the second click commits
            if rot.start_point is None:
                self._grip_rot, self._grip_rot_done = None, rot
                self._clear_grips()
            return
        if self.start_point is None and self._hot_grip is not None:
            self._start_grip_rotation(ctx)
            return
        self._ctx = ctx
        self._send(viewport, self._pointer("click", ctx))
        self._ctx = None
        self._after(viewport)

    def _grab_under(self, ctx: ToolContext, world) -> bool:
        from ingetrazo_core import transform_grab
        viewport = ctx.viewport
        if not transform_grab(self, ctx, "move"):
            return False
        self._vp_preview = TransformPose.begin(viewport, self)
        self._gather_copy_entities(ctx)
        return True

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
        rot = self._grip_rot or self._grip_rot_done
        return rot is not None and self.start_point is None

    def on_value(self, viewport, value) -> bool:
        rot = self._grip_rot or self._grip_rot_done
        if rot is not None and self.start_point is None:
            # A typed angle: exact turn while a grip is held, or the hot
            # retype of the one just made.
            done = rot.on_value(viewport, value)
            if self._grip_rot is not None and rot.start_point is None:
                self._grip_rot, self._grip_rot_done = None, rot
                self._clear_grips()
            return done
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

    def on_array_value(self, viewport, count: int, mode: str,
                       step: float | None = None) -> bool:
        """Arrays, typed right after a Move-copy (``3x``, ``/3``, ``5x10m``,
        #111): the machine decides, the copy builder stamps."""
        done = self._send(viewport, {"type": "array", "count": int(count), "mode": mode,
                                     "step": None if step is None else float(step)})["consumed"]
        self._after(viewport)
        return done

    def on_cancel(self, viewport) -> None:
        if self._grip_rot is not None:
            self._grip_rot.on_cancel(viewport)
            self._grip_rot = None
            self._clear_grips()
            viewport.update()
            return
        self._send(viewport, {"type": "cancel"})
        self._after(viewport)

    # ---- Visual preview -----------------------------------------------------
    def rubber_band_lines(self):
        if self._grip_rot is not None:
            return self._grip_rot.rubber_band_lines()
        # The geometry deforms live, so the only extra cue is the move vector
        # from the grab point to the cursor (also carries the axis-lock colour).
        grab, hover = self.grab, self.hover_point
        if grab is None or hover is None:
            return []
        segments = [(grab, hover)]
        if self._copy:
            self._fill_groups_ghost()
        if self._copy and self._base_segments:
            # Copy mode: the original stays put — preview the translated
            # COPY as a wireframe at the cursor offset.
            d = hover - grab
            segments.extend(self._base_segments.lines(delta=d.toTuple()))
        return segments

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
        edges: set[int] = set()
        for v in self._verts:
            for e in getattr(v, "edges", ()):
                edges.add(e.key)
        groups = {id(g) for g in self._groups}
        if not edges and not groups:
            return None
        return edges, groups

    def moving_face_ids(self) -> set:
        """The loose faces the drag bends — any face with a corner on a
        grabbed vertex. The viewport keeps the cursor from landing on
        them (it chased its own face and shook)."""
        if self.grab is None or self._copy:
            return set()
        return {f.key for v in self._verts
                for e in getattr(v, "edges", ()) for f in e.faces}

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

    def _grip_candidate(self, viewport, x: float, y: float):
        """The group that should show grips: the one under the cursor, when
        nothing else is selected (Move would act on the selection)."""
        pick = getattr(viewport, "pick_group", None)
        g = pick(x, y) if pick is not None else None
        if g is None or getattr(g, "billboard", False):
            return None
        sel = viewport.scene.selection
        if sel and set(sel) != {g}:
            return None
        return g

    def _grips_stay(self, viewport, obb_of, x: float, y: float) -> bool:
        from ingetrazo_core import move_box_contains
        group = self._grip_group
        sel = viewport.scene.selection
        if group is None or not self._grips or (sel and set(sel) != {group}):
            return False
        frame, lo, hi = obb_of(group)
        return move_box_contains(viewport, [_tuple(v) for v in frame], _tuple(lo), _tuple(hi), (x, y), GRIP_PX)

    def _track_grips(self, ctx: ToolContext) -> None:
        viewport = ctx.viewport
        obb_of = getattr(viewport, "_group_obb", None)
        w2p = getattr(viewport, "_world_to_pixel", None)
        cam = getattr(viewport, "camera", None)
        if obb_of is None or w2p is None or cam is None:
            return
        x, y = ctx.screen.x(), ctx.screen.y()
        g = self._grip_candidate(viewport, x, y)
        # A grip can sit off the geometry (the box of an L is bigger than
        # the L): while the cursor is on one, the grips stay.
        hot = self._grip_under(viewport, x, y)
        if g is None and hot is not None:
            self._hot_grip = hot
            return
        if g is None and self._grips_stay(viewport, obb_of, x, y):
            # Off the mesh but still inside the shown group's box: the
            # grips stay (issue #871). The camera may have moved: re-face
            # them, exactly as when the group is picked again.
            self._grips = rotation_grips(obb_of(self._grip_group),
                                         cam.eye())
            self._hot_grip = self._grip_under(viewport, x, y)
            return
        if g is not self._grip_group:
            self._grip_group = g
            self._grips = (rotation_grips(obb_of(g), cam.eye())
                           if g is not None else [])
        elif g is not None:
            # The camera may have moved: re-face the grips.
            self._grips = rotation_grips(obb_of(g), cam.eye())
        self._hot_grip = self._grip_under(viewport, x, y)

    def _grip_under(self, viewport, x, y):
        from ingetrazo_core import move_grip_hit
        return move_grip_hit(viewport, self._grips, (x, y), GRIP_PX) if self._grips else None

    def _start_grip_rotation(self, ctx: ToolContext) -> None:
        """Hand the turn to a Rotate tool set up the classic way: the
        protractor on the grip's face, its centre where the axis through
        the box centre pierces that face, the grip as the zero arm."""
        from tools.rotate import RotateTool
        viewport = ctx.viewport
        pos, face_c, normal = self._grips[self._hot_grip]
        group = self._grip_group
        rot = RotateTool()
        rot.on_activate(viewport)
        rot._groups = [group]
        rot._vp_preview = False
        begin = getattr(viewport, "begin_groups_preview", None)
        if begin is not None:
            begin([group])
            rot._vp_preview = True
        rot._gather_copy_entities(ctx)
        rot._custom_axis = QVector3D(normal).normalized()
        rot.start_point = QVector3D(face_c)
        rot.ref_point = QVector3D(pos)
        rot.hover_point = QVector3D(pos)
        self._grip_rot = rot
        viewport.flash_status(tr(
            "Rotate: move to turn, click or type the angle. Ctrl = copy"))
        viewport.update()

    def draw_overlay(self, viewport, painter) -> None:
        """The red rotation grips — the «+» marks on the box."""
        if self._grip_rot is not None or not self._grips:
            return
        from PySide6.QtCore import QPointF
        from PySide6.QtGui import QColor, QPen
        red = QColor(220, 40, 40)
        for i, (pos, _c, _n) in enumerate(self._grips):
            p = viewport._world_to_pixel(pos)
            if p is None:
                continue
            hot = i == self._hot_grip
            r = 6.0 if hot else 4.0
            painter.setPen(QPen(red, 3.0 if hot else 2.0))
            painter.drawLine(QPointF(p[0] - r, p[1]), QPointF(p[0] + r, p[1]))
            painter.drawLine(QPointF(p[0], p[1] - r), QPointF(p[0], p[1] + r))

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

    def _apply_preview(self, viewport, target_delta):
        if (target_delta - self._preview_delta).length() < 1e-12:
            return
        if self._pose is None:
            self._pose = TransformPose(self, viewport.scene, "move", self._vp_preview)
        self._pose.translate(viewport.scene, target_delta)
        if self._vp_preview:
            viewport.set_groups_preview_offset(target_delta)
        self._preview_delta = target_delta

    def _revert_preview(self, viewport):
        if self._pose is not None and self._preview_delta.length() >= 1e-12:
            self._pose.apply(viewport.scene)
        self._pose = None
        self._preview_delta = QVector3D()

    def _end_freeze(self, viewport) -> None:
        end = getattr(viewport, "end_groups_preview", None)
        if end is not None:
            end()

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
