# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Host picking, protractor inference and resident-row preview for Rotate."""
from __future__ import annotations


from PySide6.QtCore import Qt
from ingetrazo_core import Vec3 as QVector3D

from core.group import Group
from core.history import RotateGuidesCommand, rotation_matrix

from core.i18n import tr
from tools.move_host import CopyPreview
from core.guide import is_guide

from tools.protractor_host import ProtractorBase


class RotateHost(CopyPreview, ProtractorBase):
    accepts_angle_ratio = True  # VCB "3:12" (rise:run) arrives as degrees
    accepts_array = True  # VCB "3x" / "/3" after a copy: a POLAR array

    def __init__(self) -> None:
        super().__init__()
        self._groups: list[Group] = []
        self._splanes: list = []               # section planes being rotated
        self._images: list = []                # reference images being rotated
        self._moved_guides: list = []                # selected construction guides
        self._positions: list[QVector3D] = []
        self._verts: list = []
        self._sel_faces: list = []          # for copy mode (loose geometry)
        self._sel_edges: list = []
        self._base_segments: list = []      # wireframe for the copy preview
        self._groups_ghost_due = False
        self._preview_deg = 0.0
        self._orig = None                   # the preview's snapshot

    # ---- Lifecycle ----------------------------------------------------------
    def on_activate(self, viewport):
        self._send(viewport, {"type": "activate"})

    def on_deactivate(self, viewport):
        self._send(viewport, {"type": "deactivate"})
        self._axis_pick = None
        self.hover_point = None

    @property
    def cursor_plus(self) -> bool:
        """The little + beside the cursor while Ctrl has it making a COPY,
        as the Tape shows its guide mode: the status-bar flash alone was
        easy to miss."""
        return self._copy

    # ---- Keyboard -----------------------------------------------------------
    def on_key(self, viewport, key: int, modifiers) -> bool:
        if key == Qt.Key_Control:
            return self._send(viewport, {"type": "key", "key": "Control"})["consumed"]
        return super().on_key(viewport, key, modifiers)

    def _toggle_copy(self, viewport, say=True):
        self._send(viewport, {"type": "key", "key": "ToggleCopy" if say else "ToggleCopyQuiet"})

    def host_copy_changed(self, viewport, args):
        copy, say = args
        self._during["copy"] = copy
        if self._copy:
            self._revert_preview(viewport)      # the original stops swinging
            if say:
                viewport.flash_status(tr("Rotate a copy: on"))
        elif say:
            viewport.flash_status(tr("Rotate a copy: off"))
        apply = getattr(viewport, "_apply_tool_cursor", None)
        if apply is not None:
            apply()                      # the + appears or disappears now
        viewport.update()

    def on_key_release(self, viewport, key: int) -> bool:
        if key != Qt.Key_Control:
            return False
        return self._send(viewport, {"type": "key_release", "key": "Control",
                                     "tapped": getattr(viewport, "ctrl_tapped", lambda: True)()})["consumed"]

    def _pointer_event(self, kind, ctx):
        self._ctx = ctx
        try:
            self._send(ctx.viewport, self._pointer(kind, ctx))
        finally:
            self._ctx = None

    def on_click(self, ctx):
        self._pointer_event("click", ctx)

    def host_pick_rotation(self, viewport, point):
        from tools.move_host import gather_targets
        ctx = self._ctx
        groups, positions = gather_targets(ctx)
        from core.section import SectionPlane
        splanes = [p for p in viewport.scene.selection
                   if isinstance(p, SectionPlane)]
        if not splanes and not viewport.scene.selection:
            # Rotate grabs a section plane directly by its
            # frame, no pre-selection needed.
            pick = getattr(viewport, "pick_section_plane", None)
            sp = (pick(ctx.screen.x(), ctx.screen.y())
                  if pick is not None else None)
            if sp is not None:
                splanes = [sp]
                groups, positions = [], []
        from tools.move_host import gather_images
        images = gather_images(ctx)
        guides = [g for g in viewport.scene.selection
                  if is_guide(g)]
        if not (groups or positions or splanes or images or guides):
            viewport.flash_status(
                tr("Select (or click) the geometry to rotate first"))
            return
        self._groups = groups
        self._splanes = splanes
        self._images = images
        self._moved_guides = guides
        self._positions = positions
        mesh = viewport.scene.mesh
        self._verts = [v for v in (mesh.vertex_at(p) for p in positions)
                       if v is not None]
        self._vp_preview = False
        if groups and not self._verts and not positions:
            # Groups-only rotate: viewport-side preview (see MoveTool) —
            # the classic-group live rotate walked 190k vertices in
            # Python and forced a 7 s chunk rebuild EVERY frame.
            begin = getattr(viewport, "begin_groups_preview", None)
            if begin is not None:
                begin(groups)
                self._vp_preview = True
        self._gather_copy_entities(ctx)
        self._centre_plane = (ctx.world, QVector3D(self._axis()))
        self._axis_drag_armed = True
        return True

    def host_rotation_angle(self, viewport, point):
        self._note_snap(self._ctx)
        return self._commit_deg(QVector3D(*point))

    def on_hover(self, ctx):
        self._pointer_event("hover", ctx)
        ctx.viewport.update()

    def host_protractor_hover(self, viewport, _):
        ctx = self._ctx
        self.hover_point = ctx.world
        self._track_axis_drag(ctx.viewport)   # live tilt preview (issue #25)
        self._infer_plane(ctx)
        self._update_screen_metrics(ctx)
        self._note_snap(ctx)

    def host_rotation_preview(self, viewport, angle):
        self._apply_preview(viewport, angle)

    def host_swing_angle(self, viewport, _):
        if self._copy and self.hover_point is not None:
            return self._angle_to(self.hover_point)
        return self._preview_deg

    def on_release(self, viewport) -> None:
        """A real DRAG from the centre fixes the rotation axis along it
        (the classic fold gesture); a plain click keeps the inferred plane.
        The gesture lives in :class:`ProtractorBase`, shared with the
        Protractor tool."""
        if self._release_axis_drag(viewport):
            viewport.flash_status(tr("Rotation axis set along the drag"))

    def on_value(self, viewport, value):
        if isinstance(value, tuple):
            return False
        result = self._send(viewport, {"type": "value", "value": value})
        if result["redraw"]:
            viewport.update()
        return result["consumed"]

    def on_array_value(self, viewport, count, mode):
        result = self._send(viewport, {"type": "array", "count": count, "mode": mode})
        if result["redraw"]:
            viewport.update()
        return result["consumed"]

    def array_pending(self, viewport):
        from tools.rust_tool_base import _Host
        token = self._state("array_token")
        return token is not None and token == _Host(viewport).revision()

    def close_array_window(self):
        self._machine.set("close_window", None)

    def host_park_rotation(self, viewport, _):
        self._revert_preview(viewport)
        self._end_vp_preview(viewport)

    def host_end_rotation(self, viewport, _):
        self._during.update(start_point=None, ref_point=None, copy=False, array_token=None)
        self._reset()
        apply = getattr(viewport, "_apply_tool_cursor", None)
        if apply is not None:
            apply()
        viewport.update()

    # ---- Snap exclusion -----------------------------------------------------
    def snap_excluded(self):
        """The geometry swinging live, left out of the snap candidates so
        the tool never infers against itself (issue #19). Mirrors
        MoveTool.snap_excluded; in copy mode the original stays put."""
        if self.ref_point is None or self._copy:
            return None
        edges: set[int] = set()
        for v in self._verts:
            for e in getattr(v, "edges", ()):
                edges.add(e.key)
        groups = {id(g) for g in self._groups}
        if not edges and not groups:
            return None
        return edges, groups

    def moving_guide_ids(self) -> set:
        """The guides swinging live, kept out of the snap (#627)."""
        if self.ref_point is None or self._copy:
            return set()
        return {id(g) for g in self._moved_guides}

    # ---- Visual preview -----------------------------------------------------
    def rubber_band_lines(self):
        angle = (self._commit_deg(self.hover_point) if self.start_point is not None
                 and self.ref_point is not None and self.hover_point is not None else None)
        segments = self._disc_lines(angle)
        if self._copy and angle is not None:
            self._fill_groups_ghost()
            matrix = rotation_matrix(self.start_point, self._axis(), angle)
            segments.extend(self._base_segments.lines(matrix=matrix.data()))
        return segments

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

    # ---- Internals ----------------------------------------------------------
    def _snapshot(self) -> dict:
        """The rotating set exactly as it was before the preview touched it.

        The live preview used to turn everything by the DELTA of each mouse
        move and, to cancel or commit, turn it back by the total: hundreds
        of single-precision rotations that do not cancel, so the geometry
        the commit started from had already drifted (issue #163). Every
        preview frame is now computed from this copy, and reverting puts
        these exact values back."""
        snap = getattr(self, "_orig", None)
        if snap is not None:
            return snap
        from ingetrazo_core import Matrix4 as QMatrix4x4
        snap = {"xf": [], "gv": [], "v": [], "sp": [], "im": [], "gd": []}
        vp_only = getattr(self, "_vp_preview", False)
        if not vp_only:
            for group in self._groups:
                if getattr(group, "xform", None) is not None:
                    snap["xf"].append((group, QMatrix4x4(group.xform)))
                else:
                    snap["gv"].extend((group.mesh, vx, QVector3D(vx.position))
                                      for vx in list(group.mesh.vertices))
            snap["v"] = [(vx, QVector3D(vx.position)) for vx in self._verts]
        snap["sp"] = [(sp, QVector3D(sp.point), QVector3D(sp.normal))
                      for sp in self._splanes]
        snap["im"] = [(im, QVector3D(im.origin), QVector3D(im.u),
                       QVector3D(im.v)) for im in self._images]
        snap["gd"] = [RotateGuidesCommand.state(g) for g in self._moved_guides]
        self._orig = snap
        return snap

    def _pose(self, viewport, m) -> None:
        """Place the rotating set at ``m`` applied to its snapshot
        (``None`` puts the snapshot back as it was)."""
        snap = self._snapshot()
        for group, xf in snap["xf"]:
            group.xform = (m * xf) if m is not None else xf
        for gmesh, vx, pos in snap["gv"]:
            gmesh.place_vertex(vx, m.map(pos) if m is not None else pos)
        for vx, pos in snap["v"]:
            viewport.scene.mesh.place_vertex(
                vx, m.map(pos) if m is not None else pos)
        for sp, point, normal in snap["sp"]:
            sp.point = m.map(point) if m is not None else QVector3D(point)
            n2 = m.mapVector(normal) if m is not None else QVector3D(normal)
            sp.normal = n2.normalized() if n2.length() > 1e-12 else normal
        for im, origin, u, v in snap["im"]:
            im.origin = m.map(origin) if m is not None else QVector3D(origin)
            im.u = m.mapVector(u) if m is not None else QVector3D(u)
            im.v = m.mapVector(v) if m is not None else QVector3D(v)
        for st in snap["gd"]:
            RotateGuidesCommand.place(st, m)
        if snap["xf"] or snap["gv"] or snap["v"]:
            viewport.scene.version += 1

    def _apply_preview(self, viewport, target_deg: float) -> None:
        m = rotation_matrix(self.start_point, self._axis(), target_deg)
        self._pose(viewport, m)
        self._preview_deg = target_deg
        if getattr(self, "_vp_preview", False):
            viewport.set_groups_preview_matrix(m)

    def _revert_preview(self, viewport) -> None:
        if getattr(self, "_orig", None) is not None:
            self._pose(viewport, None)       # the exact values from before
            self._orig = None
        self._preview_deg = 0.0
        if getattr(self, "_vp_preview", False):
            from ingetrazo_core import Matrix4 as QMatrix4x4
            viewport.set_groups_preview_matrix(QMatrix4x4())  # park, keep freeze

    def _end_vp_preview(self, viewport) -> None:
        end = getattr(viewport, "end_groups_preview", None)
        if getattr(self, "_vp_preview", False) and end is not None:
            end()
        self._vp_preview = False

    def _make_builder(self):
        # Compatibility for extensions that assemble their own History step.
        from functools import partial
        from core.rotate_ops import rotation_command
        return partial(rotation_command, QVector3D(self.start_point), QVector3D(self._axis()),
                       self._copy, list(self._groups), list(self._positions),
                       list(self._sel_faces), list(self._sel_edges), list(self._moved_guides),
                       list(self._splanes), list(self._images))

    def host_rotation_args(self, viewport, _):
        scene = viewport.scene
        return dict(center=self.start_point.toTuple(),
                    axis=self._axis().toTuple(), copy=self._copy,
                    placements=[scene.occurrence_path(g) or g.uid for g in self._groups],
                    vertices=[str(v.key) for v in self._verts],
                    faces=[str(f.key) for f in self._sel_faces],
                    edges=[str(e.key) for e in self._sel_edges],
                    images=[scene.image_planes.index(im) for im in self._images],
                    guides=[scene.guides.index(g) for g in self._moved_guides],
                    sections=[scene.section_planes.index(sp) for sp in self._splanes])

    def _reset(self) -> None:
        self._reset_protractor()
        self._groups = []
        self._splanes = []
        self._images = []
        self._moved_guides = []
        self._positions = []
        self._verts = []
        self._sel_faces = []
        self._sel_edges = []
        self._base_segments = []
        self._groups_ghost_due = False
        self._preview_deg = 0.0
        self._orig = None       # the preview's snapshot (see _snapshot)
