# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Push/Pull tool: extrude a face along its normal.

UX (the usual push/pull convention):
- Hover a face; the cursor picks the front-most face under it.
- First click: lock onto that face and start a drag along its normal.
- Subsequent mouse motion slides the extrusion preview (wireframe of
  the future box) along the normal axis. Length is shown near the
  midpoint, same overlay as the line tool.
- Second click commits at the current distance.
- Typing a number + Enter (VCB) commits at exactly that distance,
  preserving the current direction's sign; a negative value ("-2")
  reverses it, and unit suffixes ("30cm", "1500mm") are understood.
- **Ctrl** = push/pull a copy: the start face stays in place (a slab
  division) and the extrusion stacks as a new segment — how floors are
  stacked.
- **Double-click** repeats the last committed distance on the face under
  the cursor (with Ctrl, as a copy).
- **Several faces selected**: pushing or pulling any one of them moves them
  all the same distance, each along its own normal, as one undo step.
- Esc cancels without committing.

Commit pipeline (the deterministic root-fix, no case tree):
1. Orient the mesh outward (entry invariant; no-op on open sheets).
2. Build the *naive* extrusion: consumed/kept base, moved cap (with holes),
   one wall quad per boundary/hole edge — even where a quad lands on an
   existing face's plane.
3. Stitch connectivity (T-junctions, collinear vertices).
4. On a solid: deterministically rebuild every touched plane (seams, overlaps,
   cracks) from its edges via ``Store::rebuild_push_planes``; on a flat sheet, fall back
   to the seeded coplanar merge (no outward side exists there).
5. Orient outward again — every committed solid upholds the invariant.

A bordered (embedded) base face is consumed either direction — pushing in
carves a recess / step / through-hole (the far face is punched when the push
reaches it); pushing out grows the solid. A free-standing face keeps its base
as the cap of the new box.
"""
from __future__ import annotations
from core.element_set import ElementSet

import math
import time

from PySide6.QtCore import Qt
from ingetrazo_core import Vec3 as QVector3D, push_sweep_preview, push_guide_reference
from views import input_state

from core.geometry import Face
from core.i18n import tr
from core.mesh import _STITCH_TOL
# MeshFace is core.mesh's Face — the class scene meshes are built from. The
# geometry.Face imported above is a DIFFERENT legacy class; an isinstance
# against IT is always False, so the preselection arm checks MeshFace.
from core.mesh import Face as MeshFace
from core.orient import orient_outward
from core.topology import _key
from tools.base import ToolContext, ctrl_clause
from tools.rust_tools import RustTool, _mods
from core.units import fmt_len
from core import push_scans
from core.push_pull import (  # noqa: E402,F401  (moved; tests and extensions import them here)
    PushPullOperation,
    _GroupScene,
    _MIN_EXTRUDE,
    _face_shape,
    _loops_overlap_2d,
    _made_since,
    _paint_of,
    _project_loop_2d,
)


class PushPullTool(PushPullOperation, RustTool):
    kind = "pushpull"
    _READ_DURING = ()

    def __init__(self):
        RustTool.__init__(self)
        PushPullOperation.__init__(self)
        self._ctx = None

    def busy(self):
        return self.dragging

    def host_reading(self, viewport, args):
        return {"dragging": self.dragging, "distance": self.extrusion,
                "has_base": self.base_face is not None, "mode": self._keep_mode,
                "repeat": type(self).last_distance}

    def _pointer_event(self, kind, ctx):
        previous, self._ctx = self._ctx, ctx
        try:
            return self._send(ctx.viewport, self._pointer(kind, ctx))
        finally:
            self._ctx = previous

    def on_click(self, ctx):
        self._pointer_event("click", ctx)

    def on_hover(self, ctx):
        self._pointer_event("hover", ctx)

    def on_double_click(self, ctx):
        self._pointer_event("double_click", ctx)

    def on_release(self, viewport):
        mods = _mods(input_state.modifiers(viewport, query=True)) if self.dragging else 0
        self._send(viewport, {"type": "release", "mods": mods})

    name = "Push / Pull"

    #: P is the usual key for this tool (Marco, 2026-09-10). U, the one
    #: IngeTrazo used until today, stays as a second
    #: shortcut on this same action — a decade of muscle memory is worth a
    #: line of code, and it costs nothing: P is what the card says.
    shortcut = "P"

    shortcut_alt = "U"

    description = (
        "Push or pull a face to extrude it into a solid or cut it "
        "back.")

    uses_snap = False  # picks a face to extrude; no snap markers

    hover_group_edges = True  # …but its distance infers to a group's edge too

    vcb_label = "Distance"

    # Preview lines in the normal edge colour, not the loose orange rubber band,
    # and depth-tested so the forming box hides its own back edges.
    wireframe_color = (0.13, 0.17, 0.23, 1.0)

    wireframe_depth_tested = True

    # The sweep is an OVERLAY, with no material around it to hide anything: a
    # recess drew all four of its walls, and the ones between the eye and the
    # pocket came out back-side-on, in back-face blue, over the shape the user
    # is forming (Marco, 2026-08-27). Real geometry never shows that because
    # the solid hides them. Culling the faces that point away is the same
    # result, and it costs a dot product each.
    preview_cull_back = True

    # ---- Lifecycle ----------------------------------------------------------
    def host_activate(self, viewport, args) -> None:
        self._reset()
        self._keep_mode = False      # the modifier arms ONE operation
        # The .skp preselection workflow: activated with EXACTLY one
        # Face of the loose mesh selected, the tool arms on it — the first
        # click anywhere starts the drag there, not on whatever sits under
        # the cursor. Anything else (nothing, several items, an edge, a
        # group) leaves the tool idle, as before.
        sel = viewport.scene.selection
        if len(sel) > 1:
            # Several faces: no single arm (which one would the cursor
            # measure along?), but a drag started on any of them takes them
            # all along — see _take_extras.
            faces = self._selected_faces(viewport.scene)
            if len(faces) > 1:
                viewport.flash_status(tr(
                    "{n} faces selected — push or pull one of them to move "
                    "them all", n=len(faces)), 5000)
            return
        if len(sel) != 1:
            return
        item = next(iter(sel))
        # MeshFace, not the geometry.Face imported at top: the mesh's Face
        # and the legacy one are different classes, and the loose mesh only
        # ever holds the former.
        if not isinstance(item, MeshFace) or item not in viewport.scene.mesh.faces:
            return
        self.hovered_face = item
        self._hover_group = None
        self._selected_arm = True
        viewport.set_hover(item)

    def host_deactivate(self, viewport, args) -> None:
        self._revert_preview(viewport)
        viewport.set_hover(None)
        viewport.set_suppressed_faces(set())
        self._reset()

    # ---- Spatial input ------------------------------------------------------
    def drag_plane(self, viewport):
        """Mid-drag, the plane through the anchor that CONTAINS the push
        axis and faces the camera — the cursor ray always hits it, so the
        viewport keeps delivering hovers and clicks wherever the cursor
        goes. The ground it used otherwise stops answering above the
        horizon (or when the ray merely grazes it), and then the box froze
        mid-pull and the click on its top was lost: «hago clic aquí para que
        quede fijado… no me deja, tengo que venirme al lateral» (Rafael,
        2026-09-16, camera at ground level). The distance itself is still
        read off the pixel (``_project_to_lock_line``); the plane only has
        to exist. Looking straight along the axis, where no plane contains
        it face-on, the base plane answers as before."""
        if not self.dragging or self._anchor is None or self._normal is None:
            return None
        cam = getattr(viewport, "camera", None)
        if cam is None:
            return None
        forward = cam.target - cam.eye()
        n = self._normal
        side = forward - n * QVector3D.dotProduct(forward, n)
        if side.length() < 1e-6:
            return None
        return QVector3D(self._anchor), side.normalized()

    def host_hover_face(self, viewport, args):
        ctx = self._ctx
        self._inference_point = None
        self._inference_kind = None
        # The armed preselection outranks the cursor: an idle frame
        # elsewhere must not steal it. A face gone from the mesh (undo
        # took it) disarms and falls through to a normal pick.
        if self._selected_arm and self.hovered_face is not None:
            if self.hovered_face in viewport.scene.mesh.faces:
                viewport.set_hover(self.hovered_face)
                return
            self._selected_arm = False
        # A closed group's face is only ever refused: a big lazy mesh is
        # not built to learn that (#1092). A face that can be pushed is
        # the real one.
        from views.array_bake import LightFace, reading_only
        with reading_only():
            self.hovered_face, self._hover_group = viewport.pick_face_any(
                ctx.screen.x(), ctx.screen.y())
        if self._hover_group is None and isinstance(self.hovered_face,
                                                    LightFace):
            self.hovered_face = self.hovered_face.real()
        # Shade the face that would be pushed, so the target
        # is unmistakable before clicking. A face inside a group that is not
        # open gets NO shading: it is not going to be pushed, and promising
        # it with a highlight is how the wrong thing gets modified.
        viewport.set_hover(None if self._hover_group is not None
                           else self.hovered_face)
        return

    def host_preview(self, viewport, args):
        self._preview_at(viewport, self._ctx)
        return self.dragging

    def _preview_at(self, viewport, ctx):
        if self.base_face is None or self._anchor is None:
            return
        if self._drop_stale_stage(viewport):
            return
        # Only the prism translation is undone per frame — arithmetic, so
        # inference never sees the forming solid's moving points. A staged
        # result stays put (restoring it every frame is the snapshot cost the
        # stage exists to avoid); the inference skips its geometry instead.
        self._revert_prism_preview(viewport)
        inferred = self._infer_reference_distance(ctx)
        if inferred is not None:
            self.extrusion = inferred
        else:
            grab = self._grab if self._grab is not None else self._anchor
            projected = viewport._project_to_lock_line(
                grab, self._normal, ctx.screen.x(), ctx.screen.y()
            )
            self.extrusion = QVector3D.dotProduct(
                projected - grab, self._normal)
        self._clamp_extrusion(viewport)
        # A clean prism extend/shrink is previewed by MOVING the cap, so the
        # model itself shows the result and nothing of the old shape is left
        # standing. Any other attached face is STAGED: the real commit applied
        # once, then stretched — walls merging and old edges going live, as
        # the commit will leave them. A free face (nothing to merge with) and
        # a Ctrl-copy show the sweep as an overlay — see _build_light_faces.
        # Several faces at once always show the overlay: a prism or staged
        # preview edits the model under the OTHER faces' feet. A view that
        # draws the mesh as an outline (the native one) shows it too.
        if self._extra_faces or getattr(viewport, 'mesh_outline_preview', False):
            self._show_light_preview(viewport)
        elif self._prism_cap and not self._keep_base:
            self._show_prism_preview(viewport)
        elif self._attached and not self._keep_base:
            self._show_staged_preview(viewport)
        else:
            self._show_light_preview(viewport)

    def host_begin(self, viewport, pick):
        ctx = self._ctx
        face, group = (viewport.pick_face_any(ctx.screen.x(), ctx.screen.y()) if pick
                       else (self.hovered_face, self._hover_group))
        if face is None:
            return False
        if group is not None:
            self._refuse_closed_group(viewport)
            return False
        self.base_face = face
        self._take_extras(face, viewport.scene)
        if not self._ensure_store(viewport.scene):
            self._refuse_python_mesh(viewport)
            return False
        face = self.base_face
        self.extrusion = 0.0
        self.dragging = True
        self._group = group
        self._selected_arm = False
        target = self._target_scene(viewport.scene)
        orient_outward(target.mesh, only=(face,))
        self._drag_pre_oriented = True
        self._anchor = face.centroid()
        self._normal = self._outward_normal(face, target.mesh)
        self._grab = self._grab_point(viewport, ctx)
        self._attached, self._prism_cap = self._classify_base(target)
        self._cap_positions = self._cap_loop_positions(face)
        self._prism_verts = self._cap_vertices(target)
        self._compute_inward_limit(target)
        self._compute_plane_levels(target)
        self._preview_snapshot = None
        if not pick:
            viewport.set_hover(None)
            viewport.update()
        return True

    def host_close_retype(self, viewport, args):
        self._last = None

    def host_tiny_push(self, viewport, args):
        viewport.flash_status(tr("Too small a push — drag further or type a distance"), 4000)

    def host_commit(self, viewport, distance):
        if distance is not None:
            self.extrusion = distance
            self._clamp_extrusion(viewport)
        self._commit(viewport)

    def _grab_point(self, viewport, ctx: ToolContext) -> QVector3D | None:
        """The clicked spot on the face's plane, or ``None`` (the centroid
        then stands in).

        The drag is measured along the line through this point: the cursor
        stays on it. Through the centroid instead, a click near a corner of
        a 12 m slab read -3.39 m for a 0.3 m pull, and a ring's centroid lies
        in its hole -- 1 m of cursor gave 0.13 m walls (tutorial hunt 2,
        A13/B5)."""
        hit = getattr(viewport, "_ray_hit_plane", None)
        if hit is None or self._anchor is None or self._normal is None:
            return None
        return hit((ctx.screen.x(), ctx.screen.y()),
                   (self._anchor, self._normal))

    def _refuse_python_mesh(self, viewport) -> None:
        self.base_face = None
        self._extra_faces = []
        self._extra_normals = []
        viewport.flash_status(tr("Push refused: the geometry kernel could not take "
                                 "this mesh (left it unchanged)"), 6000)

    def host_restore_retype(self, viewport, args):
        """Recover the last push's handles only while its undo entry is still on top."""
        last = self._last
        if last is None:
            return None
        stack = getattr(viewport.history, "undo_stack", None)
        if not stack or stack[-1] is not last["cmd"]:
            self._last = None
            return None
        viewport.history.undo()
        # Replay the push as a drag at the typed distance: restore the state
        # _commit cleared, neutralize the preview bookkeeping, then commit.
        # _anchor/_attached/_prism_verts stay cleared — the commit path
        # never reads them.
        self.base_face = last["face"]
        self._group = last["group"]
        self._keep_base = last["keep_base"]
        self._prism_cap = last["prism_cap"]
        self._normal = last["normal"]
        self._cap_positions = last["cap_positions"]
        self._drag_pre_oriented = last["pre_oriented"]
        self._limit_in = last["limit_in"]
        self._extra_faces = last["extras"]
        self._extra_normals = last["extra_normals"]
        self.dragging = True
        self.extrusion = 0.0
        self._preview_snapshot = None
        self._prism_applied = 0.0
        self._prism_verts = []
        self._light_faces = []
        self._light_wires = []
        self._infer_cache = None
        self._refused = False
        self._topped_out = False
        return last["sign"]

    def host_cancel(self, viewport, args):
        self._light_faces = []
        self._light_wires = []
        self._revert_preview(viewport)
        viewport.set_hover(None)
        viewport.set_suppressed_faces(set())
        self._reset()
        self._spend_keep_mode(viewport)
        viewport.update()

    def on_redo(self, viewport) -> bool:
        """Redo mid-drag abandons the push first, then lets the redo run (False)
        — on the clean model. Run under a staged preview, it would land on the
        staged result and leave that in the model with no undo entry."""
        if self.dragging:
            self.on_cancel(viewport)
        return False

    # ---- Visual preview -----------------------------------------------------
    # A free face and a Ctrl-copy show the naive sweep as overlay faces
    # (``preview_faces``) plus its own wireframe (``rubber_band_lines``); the
    # commit replaces both with the real stitched geometry. A prism or staged
    # preview shows the model itself and leaves both empty.
    def rubber_band_lines(self):
        return self._light_wires

    def preview_faces(self):
        return self._light_faces

    def _build_light_faces(self):
        self._light_wires = []
        if self.base_face is None or self._normal is None or abs(self.extrusion) < _MIN_EXTRUDE:
            return []
        faces = []
        for face, normal in zip((self.base_face, *self._extra_faces),
                                (self._normal, *self._extra_normals)):
            cap, walls, wires = push_sweep_preview(face, normal * self.extrusion,
                                                  self._CURVE_FACET_COS)
            faces.append(Face(cap[0], cap[1:], dict(face.attrs) if face.attrs else None))
            paint = _paint_of(face) or None
            faces.extend(Face(wall, attrs=paint) for wall in walls)
            self._light_wires.extend(wires)
        return faces

    def _show_light_preview(self, viewport) -> None:
        """Drop any applied preview and show the sweep as an overlay instead.
        The mesh is left untouched, so the scene version does not move and the
        consolidated buffers are not touched either — the frame costs one small
        preview upload. A held stage is parked first (a no-op once parked):
        a Ctrl tap mid-drag must find it again, not pay for a new one."""
        self._park_stage(viewport)
        self._revert_preview(viewport)
        self._light_faces = self._build_light_faces()
        # Hiding the base face is what lets a drag AWAY FROM THE EYE read as
        # a recess opening instead of a cap buried behind the base. Toward
        # the eye, the sweep covers the base anyway and the frame is
        # identical either way — worth distinguishing, because suppression
        # is the one expensive thing left in a drag frame: a group with a
        # hidden face cannot use its cached chunk, so all of its faces are
        # bucketed in Python (measured at 317 ms cold on the 3054-face
        # barbecue, 13 ms once the triangulation memo is warm). A Ctrl-stack
        # keeps its base, so it never hides it.
        #
        # "Away from the eye" is the CAMERA's question, not the normal's.
        # It used to be ``extrusion < 0`` — into the material along the
        # outward normal — which is the same thing only while the normal
        # points at the viewer. Looking at a face's BACK (an inverted face,
        # or a wall seen from inside the room), a push away from the eye is
        # positive, the base stayed, and the pocket formed invisibly behind
        # it while the commit worked («no se ve cuando hago push para
        # adentro cuando tengo la cara invertida», Marco, 2026-09-18).
        recessing = (self._light_faces and self.base_face is not None
                     and self._attached and not self._keep_base
                     and self._sweeps_away_from_eye(viewport))
        viewport.set_suppressed_faces({self.base_face} if recessing else set())
        viewport.update()

    def _sweeps_away_from_eye(self, viewport) -> bool:
        """Does the sweep move the cap away from the camera — behind the
        base face, where the base would hide it? Without a camera to ask
        (tests, the bridge) the normal's sign answers as it always did."""
        cam = getattr(viewport, "camera", None)
        eye_fn = getattr(cam, "eye", None)
        if eye_fn is None or self._anchor is None or self._normal is None:
            return self.extrusion < 0.0
        off = self._normal * self.extrusion
        return QVector3D.dotProduct(off, self._anchor - eye_fn()) > 0.0

    def _refuse_closed_group(self, viewport) -> None:
        """A face inside a group that has not been opened is not pushable.

        The tool used to push it anyway — an instance even opened a short
        editing session behind your back and shared the result to every copy
        (2026-06-10). In a real drawing that reads as
        the model changing where you did not point: a group is a box, and you
        open a box before reaching inside (Marco, 2026-09-10) — the usual
        convention: double-click the group or component to edit it first.

        Refusing has to SPEAK: a tool that does nothing without a word is the
        one that reads as broken.
        """
        viewport.flash_status(tr(
            "That face belongs to a group — double-click to open it first, "
            "then push inside"), 4000)

    def _show_prism_preview(self, viewport) -> None:
        """Preview a prism extend/shrink by translating the cap in the MODEL,
        the classic way — the walls follow it and the old shape simply
        is not there any more.

        The overlay cannot do that. It can only ADD faces on top, so pushing
        the end of a block left the original cap's edges cutting across the
        block being formed, and pushing in left the whole original block
        standing around the pocket (Marco, 2026-08-27, with a reference
        modeller beside it for comparison).

        Reversible by ARITHMETIC, not by a snapshot: the frame moves the cap
        by the delta since the last one, and the revert moves it back by the
        total. Snapshots are what made the mesh-mutating preview too slow to
        keep — measured on this machine, capture+restore is 11 ms a frame at
        3k faces and 151 ms at 40k, where translating a cap is a handful of
        points. ``_prism_cap`` is exactly the case where a translation IS the
        operation, so the two always agree.
        """
        self._light_faces = []
        self._light_wires = []
        viewport.set_suppressed_faces(set())
        delta = self.extrusion - self._prism_applied
        if abs(delta) > 1e-12:
            self._move_cap(viewport, delta, self.extrusion)
        viewport.update()

    def _cap_vertices(self, scene) -> list:
        """The Vertex OBJECTS the cap loop stands on, resolved once."""
        keys = {_key(p) for p in self._cap_positions}
        mesh = self._group.mesh if self._group is not None else scene.mesh
        near = push_scans.vertices_at_keys(mesh, self._cap_positions)
        return [v for v in (mesh.vertices if near is None else near)
                if _key(v.position) in keys]

    #: A stage (one pipeline run, snapshot work included) slower than this, in
    #: seconds, ends staging for the drag: leaving every parked interval shows
    #: the overlay, and a release outside them runs the pipeline as before.
    #: One pause per drag on a big model instead of one per reversal.
    _SLOW_STAGE_S = 0.25

    def _show_staged_preview(self, viewport) -> None:
        """Preview an attached push by showing the REAL result: the commit's
        own ``_mutate`` applied to the model once ("stage"), then only its new
        cap vertices slid along the normal per frame ("stretch").

        The overlay cannot shorten a wall, merge it with a coplanar riser or
        take an old edge away, so pushing half of a split top previewed a box
        standing on the old shape, and the shape changed on release (#434).
        Running the pipeline every frame did show it, at ~0.3 s a frame.

        The slide is exact while the cap crosses no "event level" (_stage
        works them out): every face holding a moving vertex is the cap or a
        wall containing the push direction, which stays planar as its
        vertices slide — the _swept_by_push argument. Leaving that interval
        parks the stage and restores the one parked on the cursor's side, or
        stages anew (_enter_stage). A refused or failed stage, or any new one
        once staging proved slow, shows the overlay instead."""
        self._light_faces = []
        self._light_wires = []
        d = self.extrusion
        if abs(d) < _MIN_EXTRUDE:
            # Nothing to show, as the overlay. Parked, not dropped: the
            # cursor is on its way to the other side and may come back.
            self._park_stage(viewport)
            self._revert_preview(viewport)
            viewport.set_suppressed_faces(set())
            viewport.update()
            return
        sign = 1.0 if d > 0.0 else -1.0
        if self._stage_refusal is not None and self._stage_refusal[0] != sign:
            self._stage_refusal = None
        if not self._stage_holds(d) and not self._enter_stage(viewport, sign):
            self._show_light_preview(viewport)
            return
        self._slide_stage(viewport, d)
        # The base face is genuinely gone from the staged model.
        viewport.set_suppressed_faces(set())
        viewport.update()

    def _enter_stage(self, viewport, sign: float) -> bool:
        """The cursor is outside the current stage's interval (or nothing is
        staged): park the current stage, then restore the one parked on this
        side if it holds the distance, else stage anew — unless staging
        proved slow, or this direction was refused at a distance not yet
        doubled. False = show the overlay (the model is left as it was; the
        overlay's revert cleans it)."""
        d = self.extrusion
        self._park_stage(viewport)
        rec = self._stage_cache.get(sign)
        if rec is not None and self._interval_holds(d, rec["d"], rec["lo"],
                                                    rec["hi"]):
            self._unpark_stage(viewport, rec)
            return True
        refusal = self._stage_refusal
        if self._stage_slow or (refusal is not None
                                and abs(d) < 2.0 * refusal[1]):
            return False
        if self._stage(viewport):
            return True
        self._stage_refusal = (sign, abs(d))
        return False

    def _stage(self, viewport) -> bool:
        """Apply the commit's mutation at ``extrusion`` to the clean model
        (capturing it on the drag's first stage, restoring it when the model
        is staged) and resolve what a frame may slide. False — with the clean
        model back — when the push is refused, raises, or would not stretch
        cleanly. A stage slower than _SLOW_STAGE_S makes the drag slow."""
        scene = viewport.scene
        mesh = self._target_mesh(scene)
        t0 = time.perf_counter()
        try:
            if self._preview_snapshot is None:
                self._preview_snapshot = mesh.capture_state()
                self._preview_selection = ElementSet(scene.selection)
            elif self._stage_d is not None:
                self._restore_clean(viewport)
            self._mutate(scene)
            ok = not self._refused and self._resolve_stage(mesh)
        except Exception:
            # The commit is guarded by History.execute's rollback; the stage
            # is not, and a pipeline error must not escape a mouse move.
            ok = False
        # The whole pause the user sat through, snapshot work included.
        if time.perf_counter() - t0 > self._SLOW_STAGE_S:
            self._stage_slow = True
        self._refused = False          # the commit decides that for itself
        if not ok:
            if self._preview_snapshot is not None:
                self._restore_clean(viewport)
            return False
        self._stage_d = self._stage_applied = self.extrusion
        self._stage_serial = (id(mesh), mesh._mut_serial)
        scene.version += 1
        return True

    def _resolve_stage(self, mesh) -> bool:
        """After a stage: the moving vertices (new, on the cap plane), the
        open interval ``(lo, hi)`` the slide is exact in, and the new
        faces/edges. False when a face holding a moving vertex would bend.

        The interval is bounded by the nearest event levels: the base plane
        (a reversal), the parallel faces over/under the base (a through
        punch, a flush landing, an overhang), the inward limit, and every
        non-moving vertex of a face the cap drags (a wall's bottom, a
        neighbour's top, a ledge). A stage sitting ON a level is exact only
        at its own distance."""
        snap = self._preview_snapshot
        n, anchor, d = self._normal, self._anchor, self.extrusion
        made = _made_since(mesh, snap, "vertex")
        movers = [v for v in (mesh.vertices if made is None else made)
                  if v.key not in snap["vpos"]
                  and abs(QVector3D.dotProduct(v.position - anchor, n) - d)
                  < _STITCH_TOL]
        moving = set(movers)
        levels = [0.0, *self._plane_levels]
        if self._limit_in is not None:
            levels.append(-self._limit_in)
        # The faces each mover drags that keep a fixed vertex (the cap and
        # anything else wholly on the cap plane just translate with it).
        anchored: dict = {v: [] for v in movers}
        for f in {f for v in movers for f in v.faces()}:
            rest = [w for lp in (f.loop, *f.hole_loops) for w in lp
                    if w not in moving]
            if not rest:
                continue
            levels.extend(QVector3D.dotProduct(w.position - anchor, n)
                          for w in rest)
            for lp in (f.loop, *f.hole_loops):
                for w in lp:
                    if w in moving:
                        anchored[w].append(f)
        if any(abs(lv - d) <= _STITCH_TOL for lv in levels):
            # On a level: valid at exactly this distance, nothing to slide.
            self._stage_lo = self._stage_hi = d
        else:
            # No movers is right for a through punch (no cap, the same result
            # at any depth). But a base out of true by more than the tolerance
            # has a cap whose corners all miss the plane: nothing would slide,
            # and every distance would commit this frame's, the value typed
            # silently ignored. New vertices within the base's own spread of
            # the plane tell that cap from a punch.
            if not movers:
                spread = max((abs(QVector3D.dotProduct(p - anchor, n))
                              for p in self._cap_positions), default=0.0)
                if any(abs(QVector3D.dotProduct(v.position - anchor, n) - d)
                       <= spread + _STITCH_TOL
                       for v in mesh.vertices if v not in snap["vpos"]):
                    return False
            rates = [self._slide_rate(anchored[v], n) for v in movers]
            if any(r is None for r in rates):
                return False              # a face it drags would bend
            self._stage_lo = max((lv for lv in levels if lv < d),
                                 default=-math.inf)
            self._stage_hi = min((lv for lv in levels if lv > d),
                                 default=math.inf)
            self._stage_verts = movers
            self._stage_pos = [(v.position_tuple() if "_store" in mesh.__dict__ else QVector3D(v.position), r)
                               for v, r in zip(movers, rates)]
        made_f = _made_since(mesh, snap, "face")
        made_e = _made_since(mesh, snap, "edge")
        if made_f is not None and made_e is not None:
            # Read by membership only: the same elements, found by key.
            self._stage_new = ElementSet(made_f) | set(made_e)
        else:
            self._stage_new = (
                (ElementSet(mesh.faces) - ElementSet(snap["faces"]))
                | (set(mesh.edges) - set(snap["edges"])))
        return True

    @staticmethod
    def _slide_rate(faces, n: QVector3D):
        """How a moving vertex travels per unit of push, keeping every face
        in ``faces`` (the ones it drags that keep a fixed vertex) flat: the
        push normal when they all contain it (a wall), otherwise the line
        the slanted face shares with the others, scaled to keep pace with
        the cap — the crossing of a cap rim with a slope runs down the
        slope's edge (#434 follow-up). None when no such line exists."""
        normals: list = []
        for f in faces:
            m = f.normal().normalized()
            if all(QVector3D.crossProduct(m, o).length() > 1e-3
                   for o in normals):
                normals.append(m)
        nn = n.normalized()
        slanted = [m for m in normals
                   if abs(QVector3D.dotProduct(m, nn)) >= 1e-3]
        if not slanted:
            return QVector3D(n)           # walls only: straight along n
        if len(normals) == 1:
            m = normals[0]
            line = nn - m * QVector3D.dotProduct(nn, m)
        else:
            line = QVector3D.crossProduct(normals[0], normals[1])
        along = QVector3D.dotProduct(line, nn)
        if line.length() < 1e-9 or abs(along) < 1e-3 * line.length():
            return None                   # the line runs in the cap plane
        rate = line * (QVector3D.dotProduct(n, nn) / along)
        if any(abs(QVector3D.dotProduct(m, rate)) > 1e-6 * rate.length()
               for m in normals):
            return None                   # 3+ faces with no common line
        return rate

    def _park_stage(self, viewport) -> None:
        """Keep the current stage for a return trip: its fields plus a
        snapshot of the staged model, captured the first time it is parked —
        its topology never changes, and re-entry re-places the movers
        absolutely. One per side of the base plane (no interval contains 0,
        a level); a newer stage replaces the older one."""
        if self._stage_d is None or self._stage_rec is not None:
            return
        self._stage_rec = {
            "d": self._stage_d, "lo": self._stage_lo, "hi": self._stage_hi,
            "verts": self._stage_verts, "pos": self._stage_pos,
            "new": self._stage_new, "applied": self._stage_applied,
            "state": self._target_mesh(viewport.scene).capture_state(),
            "sel": set(viewport.scene.selection),
        }
        self._stage_cache[1.0 if self._stage_d > 0.0 else -1.0] = self._stage_rec

    def _unpark_stage(self, viewport, rec: dict) -> None:
        """Show a parked stage again: one restore. restore_state resets every
        captured object and the lists, so whatever the model holds now —
        clean, or the other side's stage — needs no clean restore first."""
        scene = viewport.scene
        mesh = self._target_mesh(scene)
        mesh.restore_state(rec["state"])
        scene.selection.clear()
        scene.selection.update(rec["sel"])
        self._stage_d, self._stage_lo, self._stage_hi = (
            rec["d"], rec["lo"], rec["hi"])
        self._stage_verts, self._stage_pos, self._stage_new = (
            rec["verts"], rec["pos"], rec["new"])
        self._stage_applied = rec["applied"]
        self._stage_rec = rec
        self._stage_serial = (id(mesh), mesh._mut_serial)
        scene.version += 1

    def inference_marker(self):
        """Return ``(world_point, kind)`` for the marker the viewport draws
        when the distance inference is locked onto model geometry, or
        ``None``. The kind selects the matching snap colour and label,
        including the cyan midpoint cue."""
        if not self.dragging or self._inference_point is None:
            return None
        return self._inference_point, self._inference_kind

    def inference_guide_lines(self):
        """The «level with that point» reference segment, from the geometry
        the distance inference locked onto to the moving cap, as
        ``(reference, moved_anchor, kind)`` — ``[]`` while nothing is
        engaged."""
        # The inference point FIRST: the hot-retype replay arms a drag with no
        # anchor, and the hook must stay [] there, never evaluate anchor math.
        if self._inference_point is None or not self.dragging:
            return []
        moved = self._anchor + self._normal * self.extrusion
        return [(self._inference_point, moved, self._inference_kind)]

    def value_label(self):
        """Return ``(text, midpoint_world)`` for the floating distance label.
        Uses the anchor captured at drag start, which stays fixed even while a
        prism cap deforms the real geometry live."""
        if not self.dragging or self._anchor is None:
            return None
        midpoint = self._anchor + self._normal * (self.extrusion * 0.5)
        return (fmt_len(abs(self.extrusion)), midpoint)

    def _compute_plane_levels(self, scene) -> None:
        """The levels (along the normal, both sides) of every face parallel to
        the base that overlaps it laterally — where a staged preview's
        topology changes: the far face a push punches through, a floor it
        lands flush on, an overhang a pull runs into. Stored in
        ``self._plane_levels``; see _resolve_stage."""
        self._plane_levels = []
        if (not self._attached or self._prism_cap or self.base_face is None
                or self._normal is None):
            return                # never staged: the overlay or the prism
        n = self._normal
        nn = n.normalized()
        base_xy = _project_loop_2d(self.base_face.vertices, nn)
        near = push_scans.parallel_overlapping(scene.mesh, nn, base_xy)
        for g in (scene.faces if near is None else near):
            if g == self.base_face:
                continue
            if abs(QVector3D.dotProduct(g.normal().normalized(), nn)) <= 0.999:
                continue  # not parallel
            level = QVector3D.dotProduct(g.centroid() - self._anchor, n)
            if abs(level) <= _STITCH_TOL:
                continue  # the base's own plane: level 0 is always an event
            if _loops_overlap_2d(base_xy, _project_loop_2d(g.vertices, nn)):
                self._plane_levels.append(level)

    def _reference_hidden(self, vp, world) -> bool:
        """Whether ``world`` is a reference the user cannot see: on the
        hidden side of the active section cut, or behind other geometry.

        The snap engine runs both rules over its candidates (the viewport's
        snap scene drops cut-hidden edges; ``_is_occluded`` raycasts the
        rest), but Push/Pull sets ``uses_snap = False`` and its mid-drag
        inference read vertices and edges the eye cannot reach — the extrusion
        lunged to an invisible corner as the cursor crossed it (the "jumpy
        push" reports, #270). X-ray and wireframe show everything:
        ``_is_occluded`` is already False there, and the cut hides its side
        in every style — the same rule the snaps follow."""
        from views.document_interaction import _active_cut
        sp = _active_cut(vp.scene)
        if sp is not None and sp.side(world) > 1e-6:
            return True
        occluded = getattr(vp, "_is_occluded", None)   # stub viewports in tests
        return occluded is not None and occluded(world)

    def _clear_candidates(self, vp, arr, cand, exclude, verts):
        """The rows of ``cand``, in order, that may be a reference: neither
        the base's own corners, which would pin the drag to 0, nor hidden."""
        import numpy as np
        first_clear = getattr(vp, "_first_clear", None)
        start = 0
        if first_clear is not None and len(cand):
            # What a QVector3D of each holds, as _reference_hidden asks.
            pts = (arr[cand].astype(np.float32).astype(np.float64)
                   if arr.dtype != np.float32 else arr[cand].astype(np.float64))
            while start < len(cand):
                at = first_clear(pts[start:])
                if at is None:
                    break                # the engine cannot say: one by one
                if at < 0:
                    return
                start += at
                i = int(cand[start])
                start += 1
                if _key(verts[i].position) not in exclude:
                    yield i
        for i in cand[start:]:
            i = int(i)
            p = verts[i].position
            if _key(p) not in exclude and not self._reference_hidden(vp, p):
                yield i

    def _screen_boxes(self, vp):
        """Per entry of ``_infer_cache``, its vertices' box in pixels (as far
        as they are known) for this camera and size; None for a viewport that
        cannot say which camera it has (stubs in tests)."""
        mvp = getattr(vp, "_np_mvp", None)
        if mvp is None:
            return None
        key = (mvp().tobytes(), vp.width(), vp.height())
        held = getattr(self, "_infer_boxes", None)
        if held is None or held[0] != key:
            held = self._infer_boxes = (key, {})
        return held[1]

    def _infer_reference_distance(self, ctx: ToolContext):
        """Distance making the moved face level with the model geometry under the
        cursor — the classic mid-push inference ("push until even with that
        corner / that face"). Reads the *clean* mesh (the caller reverts a prism
        preview first, the vertex cache predates any stage, and a staged
        preview's own faces/edges are skipped), so the forming solid's moving
        points never feed back. A model **vertex** within the snap threshold
        wins first (a precise corner), followed by a nearby edge's fixed
        **midpoint**. Otherwise an edge or face under the cursor is used,
        projecting its point onto the push axis (level with where you point).
        Records the engaged point in ``self._inference_point`` for the overlay
        marker. Returns ``None`` when nothing engages."""
        vp = ctx.viewport
        sx, sy = ctx.screen.x(), ctx.screen.y()
        thr = getattr(vp, "snap_threshold_px", 9.0)
        exclude = {_key(p) for p in self._cap_positions}
        best = None
        # Batch-project the candidate vertices ONCE per mesh (positions
        # cached for the whole drag — the clean mesh is constant between
        # reverts). The per-vertex ``_world_to_pixel`` walk projected the
        # WHOLE scene (~250k QMatrix4x4.map calls) on EVERY drag move —
        # 1.3 s per move, the "push sticks" report; paints starved until
        # the hand paused.
        import numpy as np
        cache = getattr(self, "_infer_cache", None)
        if cache is None:
            cache = []
            candidates = getattr(vp, "inference_meshes", None)
            meshes = (candidates(sx, sy) if candidates is not None else
                      [vp.scene.mesh] + [g.mesh for g in getattr(vp.scene, "groups", [])])
            seen = set()
            for mesh in meshes:
                if id(mesh) in seen:      # shared prototypes: project once
                    continue
                seen.add(id(mesh))
                from core.lazy_mesh import plan_of, PlannedVertices
                plan = plan_of(mesh)
                if plan is not None:
                    # A lazy mesh (experimental engine): its planned
                    # positions, the same vertices in the same order —
                    # pushing beside a plant no longer builds all of it.
                    if len(plan.pos):
                        cache.append((PlannedVertices(plan.pos),
                                      plan.pos.copy()))
                    continue
                # A COPY: a staged preview edits the live list in place, and
                # the rows of ``arr`` must keep naming the same vertices.
                verts = list(mesh.vertices)
                if not verts:
                    continue
                arr = np.array(
                    [[v.position.x(), v.position.y(), v.position.z()]
                     for v in verts])
                cache.append((verts, arr))
            self._infer_cache = cache
            self._infer_boxes = None
        proj = getattr(vp, "_project_px", None)
        boxes = self._screen_boxes(vp) if proj is not None else None
        for n, (verts, arr) in enumerate(cache):
            if proj is not None:
                # A prototype whose box on screen is far from the cursor is
                # not projected: 1 300 of them, 230 ms a move on 5 M faces.
                box = None if boxes is None else boxes.get(n)
                if box is not None and (sx + thr + 1 < box[0]
                                        or sx - thr - 1 > box[1]
                                        or sy + thr + 1 < box[2]
                                        or sy - thr - 1 > box[3]):
                    continue
                px, py, ok = proj(arr)
                if boxes is not None and box is None:
                    boxes[n] = ((px[ok].min(), px[ok].max(),
                                 py[ok].min(), py[ok].max()) if ok.any()
                                else (np.inf, -np.inf, np.inf, -np.inf))
                d2 = (px - sx) ** 2 + (py - sy) ** 2
                near = ok & (d2 <= thr * thr)
                if best is not None:
                    near &= d2 < best[0]
                near = np.flatnonzero(near)
                # Nearest first, ties in vertex order: the first that counts
                # is the one the whole scan would keep, and the rest are never
                # read. Over a million vertices fall within the radius when
                # the prototypes pile up at their local origin (#1092).
                cand = near[np.argsort(d2[near], kind="stable")]
            else:
                # Stub viewports (tests) expose only the scalar projector.
                d2 = {}
                for i, vtx in enumerate(verts):
                    pix = vp._world_to_pixel(vtx.position)
                    if pix is None:
                        continue
                    dd = (pix[0] - sx) ** 2 + (pix[1] - sy) ** 2
                    if dd <= thr * thr and (best is None or dd < best[0]):
                        d2[i] = dd
                cand = sorted(d2, key=d2.get)
            for i in self._clear_candidates(vp, arr, cand, exclude, verts):
                p = verts[i].position
                best = (float(d2[i]), QVector3D.dotProduct(
                    p - self._anchor, self._normal), QVector3D(p), "vertex")
                break
        # Tape Measure guides (issue #165): a guide POINT is a corner like
        # any vertex -- the nearest one on screen wins -- and a guide LINE
        # is tried right after, like an edge. They live in ``scene.guides``,
        # outside the mesh, so the vertex scan above never saw them.
        from views.document_interaction import shown_guides
        guides = shown_guides(vp.scene)
        for g in guides:
            if getattr(g, "direction", None) is not None:
                continue
            pix = vp._world_to_pixel(g.point)
            if pix is None:
                continue
            dd = (pix[0] - sx) ** 2 + (pix[1] - sy) ** 2
            if dd <= thr * thr and (best is None or dd < best[0]):
                best = (dd, QVector3D.dotProduct(
                    g.point - self._anchor, self._normal),
                    QVector3D(g.point), "guide_point")
        # The origin is a point like a corner (#407): every other tool snaps
        # to it, and pushing "level with the origin" is the usual way back
        # to the ground plane.
        origin = QVector3D(0.0, 0.0, 0.0)
        pix = vp._world_to_pixel(origin)
        if pix is not None:
            dd = (pix[0] - sx) ** 2 + (pix[1] - sy) ** 2
            if dd <= thr * thr and (best is None or dd < best[0]):
                best = (dd, QVector3D.dotProduct(
                    origin - self._anchor, self._normal), origin, "origin")
        if best is not None:
            self._inference_point = best[2]
            self._inference_kind = best[3]
            return best[1]

        midpoint = self._infer_edge_midpoint(vp, sx, sy, thr, exclude)
        if midpoint is not None:
            self._inference_point = midpoint
            self._inference_kind = "midpoint"
            return QVector3D.dotProduct(midpoint - self._anchor, self._normal)

        line_hit = self._infer_guide_line(vp, guides, sx, sy, thr)
        if line_hit is not None:
            on, dist = line_hit
            self._inference_point = on
            self._inference_kind = "guide_line"
            return dist

        # No corner nearby: the EDGE under the cursor — the usual "On edge"
        # while pushing (Marco's capture, 2026-09-14: the half cylinder
        # pushed until level with the slab's far edge). The point on the
        # edge nearest the cursor ray sets the distance; the base's own
        # edges are not references, nor the ones a staged preview created.
        edge = getattr(vp, "_hover_edge", None)
        project = getattr(vp, "_project_to_lock_line", None)
        if edge is not None and project is not None and edge not in self._stage_new:
            a, b = QVector3D(edge.a), QVector3D(edge.b)
            ab = b - a
            # An edge along the push axis says nothing about how far to push
            # (the guide-line rule): its nearest point clamps to an end, and
            # a box's own vertical edge snapped an inward push back to the
            # top (#839).
            along = (ab.length() > 1e-9 and abs(QVector3D.dotProduct(
                ab.normalized(), self._normal)) > 0.999)
            if (_key(a) not in exclude or _key(b) not in exclude) \
                    and ab.length() > 1e-9 and not along:
                on = project(a, ab, sx, sy)
                t = QVector3D.dotProduct(on - a, ab) / QVector3D.dotProduct(ab, ab)
                on = a + ab * max(0.0, min(1.0, t))
                dist = QVector3D.dotProduct(on - self._anchor, self._normal)
                if abs(dist) >= _MIN_EXTRUDE \
                        and not self._reference_hidden(vp, on):
                    self._inference_point = on
                    self._inference_kind = "edge"
                    return dist

        # No edge either: align to the face under the cursor (its plane).
        # Project the ray∩plane hit onto the push axis. The base face (and
        # anything coplanar with it) reads ~0 distance — guarded out so the
        # push doesn't pin to its own plane; a face the staged preview
        # created (its moving cap, a riser) would pin it to itself.
        pick = getattr(vp, "pick_face_placement", None) or vp.pick_face_any
        from views.array_bake import reading_only
        with reading_only():                # its plane is all it is asked
            face, _grp = pick(sx, sy)
        if (face is not None and face != self.base_face
                and face not in self._stage_new
                and not (_grp is None and self._borders_base(face, exclude))):
            origin, direction = vp._pixel_to_ray(sx, sy)
            if origin is not None and direction is not None:
                # In world space — a placed component's face lives in its
                # prototype's frame.
                from core.snap import face_plane_world
                fpt, fn = face_plane_world(face, getattr(_grp, "xform", None))
                fn = fn.normalized()
                denom = QVector3D.dotProduct(fn, direction)
                if abs(denom) >= 1e-6:
                    t = QVector3D.dotProduct(fn, fpt - origin) / denom
                    if t > 0:
                        hit = origin + direction * t
                        dist = QVector3D.dotProduct(
                            hit - self._anchor, self._normal)
                        if abs(dist) >= _MIN_EXTRUDE:
                            self._inference_point = hit
                            self._inference_kind = "face"
                            return dist
        self._inference_point = None
        self._inference_kind = None
        return None

    def _infer_edge_midpoint(self, vp, sx, sy, thr, exclude):
        """A visible fixed midpoint within the normal point-snap radius.

        Read nearby loose and placed edges in world space, excluding the
        cap and staged edges. Spatial prefilters avoid walking every edge
        of a large import. An edge parallel to the push axis still has a
        precise midpoint, even though an arbitrary point along it is not a
        useful distance reference.
        """
        edges = []
        for name in ("_nearby_loose_edges", "_nearby_group_edges"):
            nearby = getattr(vp, name, None)
            if callable(nearby):
                edges.extend(nearby(sx, sy, radius_px=thr))
        hovered = getattr(vp, "_hover_edge", None)
        if hovered is not None:
            edges.append(hovered)
        best = None
        for edge in edges:
            # A guide or an image border is a SimpleNamespace (unhashable):
            # never staged geometry.
            if getattr(edge, "guide", False) or (
                    type(edge).__hash__ is not None and edge in self._stage_new):
                continue
            a, b = QVector3D(edge.a), QVector3D(edge.b)
            if (a - b).lengthSquared() < 1e-18:
                continue
            if _key(a) in exclude or _key(b) in exclude:
                # The moving cap's own edges, and the side edges of the
                # solid being pushed (one end on the cap), are no reference
                # — the same as its side faces since #839.
                continue
            point = (a + b) * 0.5
            pixel = vp._world_to_pixel(point)
            if pixel is None:
                continue
            dd = (pixel[0] - sx) ** 2 + (pixel[1] - sy) ** 2
            if (dd <= thr * thr and (best is None or dd < best[0])
                    and not self._reference_hidden(vp, point)):
                best = (dd, point)
        return None if best is None else best[1]

    @staticmethod
    def _borders_base(face, base_keys) -> bool:
        """Whether *face* shares an edge with the face being pushed: its
        walls stretch or shrink with the push, so they are part of the
        operation, not a level to push to (#839, @hamzah1985: pushing a
        box's top inward, the cursor crossing the front wall snapped the
        cap back up to where the wall was). Other faces, however far,
        still engage — the classic "push until level with that face"."""
        for lp in (face.loop, *face.hole_loops):
            n = len(lp)
            for i in range(n):
                if (_key(lp[i].position) in base_keys
                        and _key(lp[(i + 1) % n].position) in base_keys):
                    return True
        return False

    def _infer_guide_line(self, vp, guides, sx, sy, thr):
        return push_guide_reference(vp, guides, (sx, sy), thr, self._anchor,
                                    self._normal, _MIN_EXTRUDE)

    @staticmethod
    def _selected_faces(scene) -> list:
        """The selected Faces of the loose mesh."""
        return [f for f in scene.selection
                if isinstance(f, MeshFace) and f in scene.mesh.faces]

    def _take_extras(self, face, scene) -> None:
        """A drag starting on one of two or more selected faces takes the
        others along; on any other face it is the ordinary single push.

        The selection is read HERE, at every drag's start, not kept from the
        activation: the pushed caps stay selected, and pushing the same
        faces again moved only the one under the cursor."""
        faces = self._selected_faces(scene)
        if len(faces) < 2 or not any(f == face for f in faces):
            self._extra_faces, self._extra_normals = [], []
            return
        mesh = scene.mesh
        # In the mesh's order, not the selection's (a set): when the faces
        # meet, which goes first decides the result.
        from core.mesh import element_identity
        place = {element_identity(f): i for i, f in enumerate(mesh.faces)}
        faces.sort(key=lambda f: place.get(element_identity(f), 0))
        self._extra_faces = [f for f in faces if f != face]
        for f in self._extra_faces:
            orient_outward(mesh, only=(f,))
        self._extra_normals = [self._outward_normal(f, mesh)
                               for f in self._extra_faces]

    # ---- Modifier: Ctrl keeps the starting face -----------------------------
    def on_key(self, viewport, key, modifiers):
        if key == Qt.Key_Control:
            return self._send(viewport, {"type": "key", "key": "Control", "mods": _mods(modifiers)})["consumed"]
        return super().on_key(viewport, key, modifiers)

    def on_key_release(self, viewport, key):
        if key != Qt.Key_Control:
            return False
        return self._send(viewport, {"type": "key_release", "key": "Control",
                                     "tapped": self.ctrl_tapped(viewport)})["consumed"]

    def host_modifier(self, viewport, values):
        self._keep_mode, self._keep_base = values

    def host_show_mode(self, viewport, enabled):
        viewport.flash_status(tr("Keep the base: on") if enabled else tr("Keep the base: off"))
        self.refresh_hint(viewport)
        viewport.update()

    def status_clause(self) -> str:
        # Short on purpose: the clause rides the same line as the hint, and
        # the classic bar spends a whole one on «Ctrl = Crear un nuevo punto
        # de partida». "Keep the base" / "move the base" is the same idea in
        # the tool's own words (base_face, _keep_base).
        return ctrl_clause(self._keep_mode, "keep the base", "move the base")
