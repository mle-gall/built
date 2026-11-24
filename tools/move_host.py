# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Viewport bindings for the Rust Move tool, including live deformation and grips."""
from __future__ import annotations

from PySide6.QtCore import Qt
from ingetrazo_core import Vec3 as QVector3D

from core.group import Group
from core.i18n import tr
from core.mesh import Edge, Face
from core.dimension import Dimension
from core.guide import Guide, is_guide
from core.history import (CompoundCommand, MoveDimensionsCommand,  # noqa: F401
                          MoveGuidesCommand)
from core.textlabel import TextLabel
from core.topology import _key
from tools.base import ToolContext
from tools.rust_tool_base import RustTool, _mods, _point_property, _tuple
from core.units import fmt_len



def _dedup(positions: list[QVector3D]) -> list[QVector3D]:
    seen = set()
    out = []
    for p in positions:
        k = _key(p)
        if k not in seen:
            seen.add(k)
            out.append(QVector3D(p))
    return out


def gather_targets(ctx: ToolContext):
    """What a transform tool acts on: ``(groups, positions)``. EVERY selected
    (or hovered) group transforms as a unit — a mixed selection carries the
    groups AND the loose geometry together (rotating a whole drawing used to
    grab only the first group). Positions are the unique loose coordinates
    from the selection or the entity under the cursor. Shared by Move,
    Rotate and Scale."""
    viewport = ctx.viewport
    entities = list(viewport.scene.selection)
    if not entities:
        group = viewport.pick_group(ctx.screen.x(), ctx.screen.y())
        if group is not None:
            return [group], []
        edge = viewport.pick_edge(ctx.screen.x(), ctx.screen.y())
        if edge is not None:
            entities = [edge]
        else:
            face = viewport.pick_face(ctx.screen.x(), ctx.screen.y())
            if face is not None:
                entities = [face]
    groups = [ent for ent in entities if isinstance(ent, Group)]
    positions: list[QVector3D] = []
    for ent in entities:
        if isinstance(ent, Edge):
            positions.extend([ent.a, ent.b])
        elif isinstance(ent, Face):
            positions.extend(ent.vertices)
    return groups, _dedup(positions)


def groups_ghost_segments(groups) -> list:
    """Every edge of ``groups`` as a world segment: the wireframe a Ctrl-copy
    previews. A component's geometry lives in the placements it holds, so the
    ghost has to swing the whole subtree — the top-level mesh of an imported
    component is often empty. It reads every mesh, so it waits until a copy is
    asked for: on the first click of a plain move it built the whole group,
    seconds and gigabytes on a big one (#1092)."""
    from core.group import iter_placements
    segments = []
    for group in groups:
        for pg, xf in iter_placements(group):
            for e in pg.mesh.edges:
                a, b = QVector3D(e.a), QVector3D(e.b)
                if xf is not None:
                    a, b = xf.map(a), xf.map(b)
                segments.append((a, b))
    return segments


def gather_dimensions(ctx: ToolContext, groups, verts) -> dict:
    """Dimensions Move acts on, each with how: the selected ones, or (with
    nothing selected) the one under the cursor. ``"line"`` — both ends
    anchored to geometry: the dimension LINE slides and the extension
    lines stretch from their vertices (the user of DriveMeca's video:
    «no se puede mover después de colocarlo»); ``"rigid"`` — a free end
    travels with the delta. A dimension whose anchors are themselves
    being moved in this very drag is left alone: it rides along with its
    vertices, and shifting its line too would move it twice."""
    viewport = ctx.viewport
    dims = [d for d in viewport.scene.selection if isinstance(d, Dimension)]
    if not dims and not viewport.scene.selection:
        pick = getattr(viewport, "pick_dimension", None)
        d = pick(ctx.screen.x(), ctx.screen.y()) if pick else None
        if d is not None:
            dims = [d]
    from core.move_ops import dimension_modes
    return dimension_modes(dims, groups, verts)


def gather_labels(ctx: ToolContext) -> list[TextLabel]:
    """Leader texts Move acts on: the selected ones, or (with nothing
    selected) the one whose text block is under the cursor — the glyphs
    overdraw all geometry, so that grab is exclusive. Their label end
    translates while the anchor stays pinned — the classic Move-on-text
    behaviour."""
    viewport = ctx.viewport
    labels = [t for t in viewport.scene.selection if isinstance(t, TextLabel)]
    if not labels and not viewport.scene.selection:
        pick = getattr(viewport, "pick_text_label", None)
        lab = (pick(ctx.screen.x(), ctx.screen.y(), rect_only=True)
               if pick else None)
        if lab is not None:
            labels = [lab]
    return labels


def gather_guides(ctx: ToolContext) -> list[Guide]:
    """Construction guides Move acts on (#627): the selected ones, or (with
    nothing selected) the guide under the cursor. A loose edge under the
    cursor wins over the guide — the edge is the finer target — while the
    guide wins over the face or group behind it. To move something else
    from a point on a guide, select it first."""
    viewport = ctx.viewport
    guides = [g for g in viewport.scene.selection if is_guide(g)]
    if guides or viewport.scene.selection:
        return guides
    pick = getattr(viewport, "pick_guide", None)
    guide = pick(ctx.screen.x(), ctx.screen.y()) if pick else None
    if guide is None or not viewport.scene.entity_selectable(guide):
        return []
    if viewport.pick_edge(ctx.screen.x(), ctx.screen.y()) is not None:
        return []
    return [guide]


def guide_ghost_segments(guides, reach: float) -> list:
    """Wireframe of guides for a copy preview: a guide line as a segment
    ``reach`` long each way (its 10 km span would end behind the camera),
    a guide point as a small cross, plus its dashed segment's line."""
    out = []
    tick = reach * 0.02
    for g in guides:
        if g.direction is not None:
            d = g.direction * reach
            out.append((g.point - d, g.point + d))
            continue
        for ax in (QVector3D(tick, 0, 0), QVector3D(0, tick, 0),
                   QVector3D(0, 0, tick)):
            out.append((g.point - ax, g.point + ax))
        if g.origin is not None:
            out.append((QVector3D(g.origin), QVector3D(g.point)))
    return out


from core.move_ops import copied_guide  # noqa: E402,F401  (Rotate imports it from here)


def gather_images(ctx: ToolContext):
    """Reference images a transform tool acts on: the selected ones, or (with
    nothing selected) the picture under the cursor. Locked images are already
    filtered out by ``pick_image_plane`` — an aligned scan should not jump
    because the user grabbed near it. Shared by Move, Rotate and Scale."""
    from core.image_plane import ImagePlane
    viewport = ctx.viewport
    images = [e for e in viewport.scene.selection if isinstance(e, ImagePlane)]
    if not images and not viewport.scene.selection:
        pick = getattr(viewport, "pick_image_plane", None)
        hit = pick(ctx.screen.x(), ctx.screen.y()) if pick else None
        if hit is not None:
            images = [hit]
    return images


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


class MoveHost(RustTool):
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
        """What the first click grabs: the selection, or what lies under the
        cursor (a group, a vertex, a guide, a label, a dimension, a section
        plane, an image). True when there is something."""
        viewport = ctx.viewport
        groups, positions = self._gather(ctx)
        labels = gather_labels(ctx)
        guides = gather_guides(ctx)
        from core.section import SectionPlane
        splanes = [p for p in viewport.scene.selection
                   if isinstance(p, SectionPlane)]
        if not splanes and not viewport.scene.selection:
            # Move grabs a section plane directly by hovering
            # its frame — no pre-selection needed. The cut follows live.
            pick = getattr(viewport, "pick_section_plane", None)
            sp = (pick(ctx.screen.x(), ctx.screen.y())
                  if pick is not None else None)
            if sp is not None:
                splanes = [sp]
                groups, positions, guides = [], [], []
        if labels and not viewport.scene.selection:
            # A click on the glyphs grabs just the text, never the
            # geometry that happens to sit behind it.
            groups, positions, guides = [], [], []
        hovered_guide = bool(guides) and not viewport.scene.selection
        if hovered_guide:
            # A click on a guide grabs the guide, not the face behind.
            groups, positions = [], []
        images = [] if hovered_guide else gather_images(ctx)
        mesh = viewport.scene.mesh
        verts = [v for v in (mesh.vertex_at(p) for p in positions)
                 if v is not None]
        dims = gather_dimensions(ctx, groups, verts)
        if dims and not viewport.scene.selection:
            # A click on a dimension's lines grabs the dimension, not
            # the geometry behind it.
            groups, positions, verts, guides = [], [], [], []
        if not (groups or positions or labels or splanes or images
                or dims or guides):
            return False  # nothing under the cursor / selected to move
        self._groups = groups
        self._positions = positions
        self._labels = labels
        self._moved_guides = guides
        self._dims = dims
        self._splanes = splanes
        self._images = images
        # The grabbed positions resolved to vertex OBJECTS once: the live
        # preview then moves these identities directly, so dragging through
        # (or onto) a coincident vertex never drags the innocent one along.
        self._verts = verts
        self._vp_preview = False
        if (self._groups and not self._verts and not self._positions):
            # Groups-only drag: viewport-side preview — the caches
            # freeze and the movers render from one-time scratch VBOs
            # at a translated MVP (the per-frame rebuild storm was the
            # 'mover un grupo demora' lag).
            begin = getattr(viewport, "begin_groups_preview", None)
            if begin is not None:
                begin(self._groups)
                self._vp_preview = True
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
            segments.extend((a + d, b + d) for a, b in self._base_segments)
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

    def _grips_stay(self, viewport, obb_of, w2p, x: float, y: float) -> bool:
        """Whether the grips on show may stay although the cursor is off
        the group's geometry and off every grip: the selection must still
        be this group (or nothing), and the cursor still inside the
        group's box as projected on screen, GRIP_PX of margin around it.
        On a thin, long object — a pipe, an elbow — the grips sit on the
        box, far from the mesh, and the way from the mesh out to one of
        them crosses a stretch where the pick finds nothing and no grip
        is near yet: the grips vanished mid-journey and none could be
        taken (issue #871). This only ever KEEPS grips that are already
        on show; acquiring a group still takes the pick, as before."""
        group = self._grip_group
        if group is None or not self._grips:
            return False
        sel = viewport.scene.selection
        if sel and set(sel) != {group}:
            return False
        frame, lo, hi = obb_of(group)
        axes = [QVector3D(a) for a in frame]
        pts = []
        for i in (0, 1):
            for j in (0, 1):
                for k in (0, 1):
                    corner = (axes[0] * (hi[0] if i else lo[0])
                              + axes[1] * (hi[1] if j else lo[1])
                              + axes[2] * (hi[2] if k else lo[2]))
                    p = w2p(corner)
                    if p is None:
                        return False
                    pts.append((p[0], p[1]))
        from ingetrazo_core import move_grip_region
        return move_grip_region(pts, (x, y), GRIP_PX)

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
        hot = self._grip_under(w2p, x, y)
        if g is None and hot is not None:
            self._hot_grip = hot
            return
        if g is None and self._grips_stay(viewport, obb_of, w2p, x, y):
            # Off the mesh but still inside the shown group's box: the
            # grips stay (issue #871). The camera may have moved: re-face
            # them, exactly as when the group is picked again.
            self._grips = rotation_grips(obb_of(self._grip_group),
                                         cam.eye())
            self._hot_grip = self._grip_under(w2p, x, y)
            return
        if g is not self._grip_group:
            self._grip_group = g
            self._grips = (rotation_grips(obb_of(g), cam.eye())
                           if g is not None else [])
        elif g is not None:
            # The camera may have moved: re-face the grips.
            self._grips = rotation_grips(obb_of(g), cam.eye())
        self._hot_grip = self._grip_under(w2p, x, y)

    def _grip_under(self, w2p, x: float, y: float):
        best, best_d = None, GRIP_PX
        for i, (pos, _c, _n) in enumerate(self._grips):
            p = w2p(pos)
            if p is None:
                continue
            d = ((p[0] - x) ** 2 + (p[1] - y) ** 2) ** 0.5
            if d <= best_d:
                best, best_d = i, d
        return best

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
    def _gather(self, ctx: ToolContext):
        vertex = self._hovered_vertex(ctx)
        if vertex is not None:
            return [], [QVector3D(vertex.position)]
        return gather_targets(ctx)

    @staticmethod
    def _hovered_vertex(ctx: ToolContext):
        """With nothing selected, a grab ON a corner (the Endpoint snap of
        loose geometry) takes that one vertex — not the edge or face it
        belongs to, which is what the pick under the cursor would give."""
        viewport = ctx.viewport
        snap = getattr(ctx, "snap", None)
        if (viewport.scene.selection or snap is None
                or snap.kind != "endpoint" or snap.context):
            return None
        return viewport.scene.mesh.vertex_at(snap.point)

    def _fill_groups_ghost(self) -> None:
        """The grabbed groups' wireframe, first thing in the copy preview."""
        if self._groups_ghost_due:
            self._groups_ghost_due = False
            self._base_segments[:0] = groups_ghost_segments(self._groups)

    def _gather_copy_entities(self, ctx: ToolContext) -> None:
        """The faces/edges copy mode duplicates, and the wireframe segments
        the copy preview carries (group wireframe, or the loose selection).
        Mirrors RotateTool._gather_copy_entities."""
        viewport = ctx.viewport
        self._sel_faces, self._sel_edges, self._base_segments = [], [], []
        self._groups_ghost_due = True     # asked of only when copying (#1092)
        cam = getattr(viewport, "camera", None)
        reach = max(float(getattr(cam, "distance", 10.0)), 1e-3)
        self._base_segments.extend(guide_ghost_segments(self._moved_guides, reach))
        sel = list(viewport.scene.selection)
        if not sel and not self._groups and not self._moved_guides:
            # Nothing selected and no group hover-picked: the click grabbed
            # the loose entity under the cursor (mirror of gather_targets).
            edge = viewport.pick_edge(ctx.screen.x(), ctx.screen.y())
            if edge is not None:
                sel = [edge]
            else:
                face = viewport.pick_face(ctx.screen.x(), ctx.screen.y())
                if face is not None:
                    sel = [face]
        self._sel_faces = [f for f in sel if isinstance(f, Face)]
        self._sel_edges = [e for e in sel if isinstance(e, Edge)]
        for f in self._sel_faces:
            for lp in (list(f.vertices), *[list(h) for h in f.holes]):
                n = len(lp)
                for i in range(n):
                    self._base_segments.append(
                        (QVector3D(lp[i]), QVector3D(lp[(i + 1) % n])))
        for e in self._sel_edges:
            self._base_segments.append((QVector3D(e.a), QVector3D(e.b)))

    def _make_copy_builder(self, scene=None):
        """The copy command for a list of offsets, kept by the array window
        so ``3x`` / ``/3`` re-lay the copies (``core.move_ops.copy_builder``,
        the implementation the protocol's ``move_selection`` runs too)."""
        from core.move_ops import copy_builder
        return copy_builder(scene, self._groups, self._sel_faces, self._sel_edges,
                            self._moved_guides)

    def _shift(self, viewport, step: QVector3D) -> None:
        """Translate the live geometry by ``step`` — every grabbed group's
        mesh AND the grabbed loose vertex objects (identity-exact: apply and
        revert stay symmetric even when the drag crosses another vertex's
        position)."""
        for group in self._groups:
            if getattr(group, "xform", None) is not None:
                from ingetrazo_core import Matrix4 as QMatrix4x4
                t = QMatrix4x4()
                t.translate(step)
                group.xform = t * group.xform   # instance: O(1)
            else:
                for v in list(group.mesh.vertices):
                    group.mesh.move_vertex(v, step)
        for v in self._verts:
            viewport.scene.mesh.move_vertex(v, step)
        for lab in self._labels:
            lab.offset = lab.offset + step
        for g in self._moved_guides:
            MoveGuidesCommand.shift(g, step)
        for dim, mode in self._dims.items():
            MoveDimensionsCommand.shift(dim, mode, step)
        for sp in self._splanes:
            sp.point = sp.point + step
        for im in self._images:
            im.origin = im.origin + step
        if self._splanes and not (self._groups or self._verts or self._labels
                                  or self._moved_guides or self._dims
                                  or self._images):
            # Only section planes ride the drag: no face moved, so nothing
            # built from the model is thrown away on each pointer move.
            viewport.scene.bump_sections()
        else:
            viewport.scene.version += 1

    def _apply_preview(self, viewport, target_delta: QVector3D) -> None:
        """Live-deform the scene so the current offset is ``target_delta`` from
        the grab point, by translating the incremental step."""
        step = target_delta - self._preview_delta
        if step.length() < 1e-12:
            return
        if getattr(self, "_vp_preview", False):
            # Groups-only drag: the viewport draws the frozen scratch copy
            # at the offset — no geometry mutation, no version churn, no
            # rebuilds. Labels/section planes ride along in Python (few).
            for lab in self._labels:
                lab.offset = lab.offset + step
            for g in self._moved_guides:
                MoveGuidesCommand.shift(g, step)
            for dim, mode in self._dims.items():
                MoveDimensionsCommand.shift(dim, mode, step)
            for sp in self._splanes:
                sp.point = sp.point + step
            for im in self._images:
                im.origin = im.origin + step
            self._preview_delta = target_delta
            viewport.set_groups_preview_offset(target_delta)
            return
        self._shift(viewport, step)
        self._preview_delta = target_delta

    def _revert_preview(self, viewport) -> None:
        """Undo the live deformation, returning geometry to its grab-time spot."""
        if getattr(self, "_vp_preview", False):
            # Viewport-side preview: no geometry was mutated — only the
            # labels/section planes moved live. Put those back.
            if self._preview_delta.length() >= 1e-12:
                step = -self._preview_delta
                for lab in self._labels:
                    lab.offset = lab.offset + step
                for g in self._moved_guides:
                    MoveGuidesCommand.shift(g, step)
                for dim, mode in self._dims.items():
                    MoveDimensionsCommand.shift(dim, mode, step)
                for sp in self._splanes:
                    sp.point = sp.point + step
                for im in self._images:
                    im.origin = im.origin + step
            self._preview_delta = QVector3D(0.0, 0.0, 0.0)
            return
        if self._preview_delta.length() < 1e-12:
            return
        self._shift(viewport, -self._preview_delta)
        self._preview_delta = QVector3D(0.0, 0.0, 0.0)

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
