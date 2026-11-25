# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Push/Pull as an operation: classify the face, find how far it may go,
and commit the extrusion, the push-through or the hole. The one
implementation behind the Push/Pull tool and the protocol's ``push_pull``
(moved out of ``tools/pushpull.py``, docs/rust-tools.md §18)."""
from __future__ import annotations

from ingetrazo_core import Vec3 as QVector3D

from core.geometry import Face
from core.i18n import tr
from core.mesh import _STITCH_TOL, PAINT_KEYS
from core.history import SnapshotMutation
from ingetrazo_core import arrangement as _arrangement
from core.orient import is_closed, orient_outward
from ingetrazo_core import plane_cycle as _canon_loop
from core.topology import _mesh_is_flat
from core.units import fmt_len
from core import push_scans
from ingetrazo_core import guarded_push, translate_prism_cap, PushStages


class _Abandoned(Exception):
    """A store refusal ends a top-out: no shorter push is tried."""


#: Repros written this session, by content: a refusal repeated on every
#: drag frame writes one file.
_REPROS: set = set()


def _write_repro(mesh, op, reason: str) -> str | None:
    """``push-refusals/<time>.json`` under the log folder: the mesh as it
    was (the store's dense arrays, the paint rows and own dicts by face
    index), the push's arguments and the refusal; logged with the refusal
    counter (``INGETRAZO_REFUSALS``). The path, or None (a repeat, or no
    writable folder)."""
    import json
    import os
    import time
    from core.store_mesh import _attribute_owner, _log_refusal
    st = mesh._store
    arrays = {k: v.tolist() for k, v in st.arrays().items()}
    faces = st.keys("face")
    index = {k: i for i, k in enumerate(faces)}

    def at(face):
        return index.get(getattr(face, "key", None))

    owner = _attribute_owner(mesh)
    data = {
        "reason": reason,
        "push": {"face": at(op.base_face), "distance": op.extrusion,
                 "keep_base": bool(op._keep_base),
                 "normal": None if op._normal is None else list(op._normal.toTuple()),
                 "extra_faces": [at(f) for f in op._extra_faces]},
        "mesh": arrays,
        "attr_table": owner["_attr_table"],
        "own_attrs": {index[k]: v for k, v in owner["_attr_own"].items() if k in index},
    }
    text = json.dumps(data, default=repr, sort_keys=True)
    digest = hash(text)
    if digest in _REPROS:
        return None
    _REPROS.add(digest)
    try:
        from core.paths import user_log_dir
        folder = user_log_dir() / "push-refusals"
        folder.mkdir(parents=True, exist_ok=True)
        path = folder / f"{time.strftime('%Y%m%d-%H%M%S')}-{len(_REPROS)}.json"
        path.write_text(text, encoding="utf-8")
    except OSError:
        return None
    log = os.environ.get("INGETRAZO_REFUSALS")
    if log:
        _log_refusal(log, "push", None, repro=str(path), reason=reason[:200])
    return str(path)


def _write_conversion_failure(reason: str) -> None:
    from core.store_mesh import _note_refusal
    try:
        _note_refusal("to_store: " + reason)
    except Exception:  # noqa: BLE001 — a count, never a failure of its own
        pass


def _project_loop_2d(loop, normal):
    return _arrangement.project_loop(loop, normal)


def _loops_overlap_2d(a_xy, b_xy) -> bool:
    """Whether two loops projected to the same plane overlap laterally: a
    representative interior point of one inside the other, or any vertex
    strictly inside. Good for the push-clamp candidates (a far cap under the
    whole loop, a floor under a corner notch); exotic offset-cross overlaps
    with no contained vertex are not the clamp's concern. Holes are ignored."""
    return _arrangement.loops_overlap(a_xy, b_xy)


class _GroupScene:
    """Scene facade aiming the push/pull machinery at a Group's isolated mesh.

    Mirrors the slice of :class:`core.scene.Scene` the commands, the stitch and
    the per-plane rebuild touch — ``mesh`` / ``faces`` / ``edges`` /
    ``selection`` / ``version`` — so the whole pipeline runs unchanged inside
    the group (its geometry is already in world coordinates). The version
    bump lands on the real scene so the viewport re-renders."""

    def __init__(self, scene, group) -> None:
        self._scene = scene
        self.mesh = group.mesh
        self.selection = scene.selection

    @property
    def faces(self):
        return self.mesh.faces

    @property
    def edges(self):
        return self.mesh.edges

    @property
    def version(self):
        return self._scene.version

    @version.setter
    def version(self, value):
        self._scene.version = value


def _made_since(mesh, snap, kind: str):
    """The elements of ``kind`` a store-backed ``mesh`` holds and its
    snapshot ``snap`` (``capture_state()``) does not, in list order — what
    ``[x for x in mesh.<kind>s if x not in snap]`` finds — or ``None`` (a
    Python mesh or snapshot, or the scans off)."""
    from core.store_mesh import StoreState
    if not isinstance(snap, StoreState):
        return None
    return push_scans.made_since(mesh, snap.snap, kind)


# The smallest extrusion that still moves geometry: anything below the mesh's
# weld resolution (1e-4, ~0.1 mm) would weld the top loop onto the base loop
# vertex-by-vertex and degenerate the wall quads. Treated as a no-op push.
_MIN_EXTRUDE = 2e-4


def _paint_of(face) -> dict:
    """The base face's paint, ready to stamp on the geometry a push creates.

    The material extrudes with the shape: pull a textured rectangle
    up and the four sides come out textured too, not bare. Only the moved cap
    inherited it here, so a pushed rectangle gave a box with one painted face
    (Marco, 2026-08-27).

    The texture's per-face ``uvw`` is deliberately NOT carried. It is a
    world→UV map fitted in the BASE's plane, so evaluating it on a wall that
    stands up out of that plane leaves the image constant along the extrusion
    — the texture smears into stripes. Dropped, each new face falls to the
    default planar projection in its OWN plane at the material's tile size,
    which is what a .skp reader draws; ``planar`` says so out loud, so the .skp
    exporter writes the default projection instead of pinning a matrix.

    The cap is not handled here: it is the base's continuation, keeps the full
    attrs (``uvw`` included), and so keeps the exact texture placement — a
    translation along the normal leaves an in-plane map unchanged.
    """
    attrs = face.attrs or {}
    paint = {k: attrs[k] for k in PAINT_KEYS if k in attrs}
    tex = paint.get("texture")
    if tex:
        paint["texture"] = {**{k: v for k, v in tex.items() if k != "uvw"},
                            "planar": True}
    return paint


def _face_shape(face) -> tuple:
    """Which way round a face is wound does not matter here: whether the
    rebuild handed back the same face, outline and holes."""
    def loop(pts):
        pts = list(pts)
        return min(_canon_loop(pts), _canon_loop(pts[::-1]))
    return (loop(face.vertices), frozenset(loop(h) for h in face.holes),
            face.interior)


class PushPullOperation:
    """The state and the commit of one push/pull: the face, how far it may
    go, the staged preview, and the command that makes it. The tool adds
    the pointer, the keys and the preview drawing."""

    # Last committed distance (signed along the base's outward normal), shared
    # across activations: double-click repeats it on another face, as
    # push/pull users expect.
    last_distance: float | None = None

    def __init__(self) -> None:
        self.hovered_face: Face | None = None
        self.base_face: Face | None = None
        self.extrusion: float = 0.0  # signed distance along normal
        self.dragging: bool = False
        # Ctrl held = "push/pull a copy": the base face stays in place (a slab
        # division), the extrusion stacks as a new segment instead of growing
        # the neighbours — the classic way to stack floors.
        self._keep_base: bool = False
        # A Ctrl TAP hands the operation over to the modifier: the starting
        # face stays put on THIS push, which is then spent (the one-shot modifier).
        self._keep_mode: bool = False
        # Deepest allowed inward push (positive, along −normal), computed at
        # drag start; None = unbounded. The "Offset limited to" clamp.
        self._limit_in: float | None = None
        # The Group whose face is being pushed (None = the loose mesh). The
        # machinery can run on a group's isolated mesh, but the GESTURE no
        # longer offers it: a face inside a group you have not opened is not
        # pushable. See _refuse_closed_group.
        self._group = None
        self._hover_group = None
        # The preselection arm: activated with EXACTLY one loose-mesh Face
        # selected, the tool locks onto it (the .skp workflow) — the
        # hover shade goes up at once, idle hovers do not steal it, and the
        # first click ANYWHERE starts the drag on it. Disarms when a drag
        # starts, on _reset (Esc, deactivate, commit), and when the armed
        # face is no longer in the mesh (undo) — the next idle hover then
        # picks normally again.
        self._selected_arm: bool = False
        # Several faces at once: a drag started on a face while two or more
        # loose-mesh Faces are selected, that face among them, pushes them
        # all — the same distance, each along its own outward normal, one
        # undo step. ``_extra_faces`` (and their outward normals) are the
        # ones riding along with the base face in the drag under way; empty
        # = the ordinary single-face push.
        self._extra_faces: list = []
        self._extra_normals: list = []
        # Whether the base face is embedded in a solid (its boundary edges are
        # shared), so an inner face pushed in is a recess (window/door pocket).
        self._attached: bool = False
        # A prism cap is every edge backing onto a perpendicular wall — the push
        # is a clean translation of the cap (walls following through shared
        # vertices) rather than a strip-by-strip extrude.
        self._prism_cap: bool = False
        self._drag_pre_oriented: bool = False
        self._anchor: QVector3D | None = None  # fixed centroid for measuring extrusion
        # Where the face was clicked, on its plane: the drag measures from
        # here (see _grab_point).
        self._grab: QVector3D | None = None
        self._normal: QVector3D | None = None
        self._cap_positions: list[QVector3D] = []  # original cap vertices
        self._infer_cache = None
        self._stages = PushStages()
        # How much of ``extrusion`` the prism preview has already applied to
        # the model, so a frame moves only the delta and the revert knows the
        # total to undo, and the very Vertex OBJECTS it moves.
        self._prism_applied: float = 0.0
        self._prism_verts: list = []
        self._plane_levels: list = []
        # The model point the distance inference is currently locked onto (a
        # corner or a face hit), drawn as a green marker by the viewport overlay.
        self._inference_point: QVector3D | None = None
        self._inference_kind: str | None = None
        # Set when the last _mutate refused the push to keep the solid valid
        # (the BIM-grade guard); _commit surfaces it in the status bar.
        self._refused: bool = False
        self._topped_out: bool = False
        # ---- Drag preview -----------------------------------------------
        # A free face and a Ctrl-copy show the naive sweep as an overlay — cap
        # plus wall quads, nothing touched in the mesh. An attached face is
        # STAGED instead (_show_staged_preview): the real pipeline runs once
        # and frames only slide its cap. Running the pipeline per mouse-move
        # cost ~0.3 s a frame on an imported barbecue.
        self._light_faces: list = []
        # Top outlines and hard risers from the extrusion preview.
        self._light_wires: list = []
        # ---- Hot retype ------------------------------------------------------
        # The just-committed push, adjustable by typing (on_value): typing a
        # distance undoes that one command and re-commits the push at the
        # typed TOTAL distance — positive keeps the committed direction,
        # negative reverses it — leaving ONE undo step however many retypes.
        # Holds the drag state _commit consumed (the mesh restore is
        # identity-preserving, so the Face object comes back valid), the
        # command that must sit on the undo stack's top, and the committed
        # direction's sign. Armed only where a push actually landed; a
        # click, Esc, a tool change or the user's own undo closes it — the
        # same window contract Rotate already has (tools/rotate.py).
        self._last: dict | None = None

    def _ensure_store(self, scene) -> bool:
        """A push runs on a store: a mesh materialised after
        another operation's refusal
        becomes store-backed first (``store_mesh.to_store``), outside any
        History step, and what this operation and the selection hold is
        carried over to the new elements. False when the store refuses the
        mesh: the push does not start."""
        from core.store_mesh import StoreMesh, StoreRefusal, to_store
        mesh = self._target_mesh(scene)
        if type(mesh) is StoreMesh:
            return True
        # The annotations on it reattach by position, as when an edit
        # context rebuilds a mesh with fresh keys (Scene._leave_level).
        root = getattr(scene, "_scene", scene)
        refresh = getattr(root, "_refresh_dimension_holds", None)
        swap = getattr(root, "_attachment_swap", None)
        if refresh is not None:
            refresh()
        refs = swap() if swap is not None else ()
        try:
            views = to_store(mesh)
        except (StoreRefusal, ValueError) as e:
            _write_conversion_failure(str(e))
            return False
        if refs:
            root._restore_attachments(refs)
        if refresh is not None:
            refresh()

        def carry(x):
            return views.get(id(x), x)
        self.base_face = carry(self.base_face)
        self.hovered_face = carry(self.hovered_face)
        self._extra_faces = [carry(f) for f in self._extra_faces]
        selection = getattr(scene, "selection", None)
        if selection is not None:
            held = [carry(x) for x in selection]
            selection.clear()
            selection.update(held)
        scene.version += 1
        return True

    def _target_scene(self, scene):
        """The scene the machinery edits: the real one, or a facade over the
        target group's isolated mesh."""
        return scene if self._group is None else _GroupScene(scene, self._group)

    def _target_mesh(self, scene):
        return self._group.mesh if self._group is not None else scene.mesh

    def _move_cap(self, viewport, delta: float, applied: float) -> None:
        """Slide the cap's own vertices by ``delta`` along the normal.

        By IDENTITY, never by position. Resolving them by position each frame
        is what warped Marco's cube (2026-08-27): pushed back until the cap
        sat flush against the far face, its key matched that face's vertices
        too, and the next frame dragged them along — the corner came out
        chamfered. ``MoveVerticesCommand`` carries the same lesson in its
        docstring; this is the cheap inverse it warns about, and holding the
        objects is what makes it safe.
        """
        mesh = self._target_mesh(viewport.scene)
        step = self._normal * delta
        store = mesh.__dict__.get("_store")
        if store is not None:
            store.move_vertices([v.key for v in self._prism_verts], step.toTuple())
            mesh._bump()
        else:
            for vertex in self._prism_verts:
                mesh.move_vertex(vertex, step)
        # Booked as the TARGET, not as a running sum: adding deltas frame by
        # frame drifts, and the revert has to undo exactly what it applied.
        self._prism_applied = applied
        viewport.scene.version += 1

    def _revert_prism_preview(self, viewport) -> None:
        """Put a translated cap back where it started."""
        if self._prism_applied:
            self._move_cap(viewport, -self._prism_applied, 0.0)

    _stage_d = property(lambda self: self._stages.distance)
    _stage_cache = property(lambda self: self._stages.cached_sides)
    _stage_new = property(lambda self: self._stages.made)
    _stage_slow = property(lambda self: self._stages.slow)
    _stage_refusal = property(lambda self: self._stages.refusal)
    _preview_snapshot = property(lambda self: self._stages.snapshot)

    _stage_verts = property(lambda self: self._stages.vertices)

    def _staged_command(self, viewport):
        """The commit of a held stage that already IS the result at
        ``extrusion``: the cap slid there, the staged model captured as the
        command's result and the clean snapshot as its undo state. Two
        snapshots and a restore instead of a pipeline run, and the commit is
        exactly the last frame. None when the pipeline must run: nothing
        staged, Ctrl, outside the interval, a stale snapshot."""
        if (self._stage_d is None or self._keep_base
                or self._drop_stale_stage(viewport)
                or abs(self.extrusion) < _MIN_EXTRUDE
                or not self._stages.holds(self.extrusion, _STITCH_TOL)):
            return None
        mesh = self._target_mesh(viewport.scene)
        self._stages.slide(mesh, viewport.scene, self.extrusion)
        cmd = SnapshotMutation.from_states(
            self._preview_snapshot, mesh.capture_state(),
            mesh=self._group.mesh if self._group is not None else None)
        # The command owns the snapshots now: nothing is restored.
        self._stages = PushStages()
        self._refused = False
        return cmd

    def _drop_stale_stage(self, viewport) -> bool:
        """Something other than this tool edited the mesh while a snapshot is
        held (File > New, a redo that bypassed on_redo): the snapshots no
        longer describe the model, so the drag ends WITHOUT restoring one.
        True when dropped."""
        if self._preview_snapshot is None:
            return False
        mesh = self._target_mesh(viewport.scene)
        if not self._stages.stale(mesh):
            return False
        viewport.set_suppressed_faces(set())
        self._reset()
        viewport.update()
        return True

    def _revert_preview(self, viewport) -> None:
        """Back to the clean model: the prism slid back, a staged model
        restored (its stage dropped, not parked). The overlay calls this every
        frame, so a model that is already clean must cost nothing: only a
        staged one is restored, and the snapshot stays for the drag."""
        self._revert_prism_preview(viewport)
        if self._drop_stale_stage(viewport):
            return
        if self._stage_d is not None:
            self._stages.restore(self._target_mesh(viewport.scene), viewport.scene)

    # ---- Internals ----------------------------------------------------------
    def _compute_inward_limit(self, scene) -> None:
        """How far the face can be pushed *into* the solid before the sweep
        would shoot past blocking geometry — the "Offset limited to" rule —
        stored in ``self._limit_in`` (``None`` = unbounded). Native
        (``core.push_scans.inward_limit``, ``pushscan::scan_inward_limit``):
        a parallel blocker behind the whole base (#1076; a face the base
        would punch through is a through-hole target, not a blocker), a
        backing wall leaning away from the push (#710: limit 0), and the
        lateral exit through a non-parallel wall, a hair short of it.
        Pushing exactly *to* a blocker is allowed: landing flush dissolves
        or opens the geometry."""
        self._limit_in = None
        if not self._attached or self.base_face is None:
            return
        n = (self._normal if self._normal is not None
             else self.base_face.normal()).normalized()
        anchor = (self._anchor if self._anchor is not None
                  else self.base_face.centroid())
        self._limit_in = push_scans.inward_limit(scene.mesh, self.base_face, n, anchor,
                                                 _STITCH_TOL)

    def _clamp_extrusion(self, viewport=None) -> None:
        if self._limit_in is not None and self.extrusion < -self._limit_in:
            self.extrusion = -self._limit_in
            if viewport is not None:
                # The usual name is "Offset limited to", where *offset* is
                # the push distance — but here Offset is another tool (F), and
                # in Spanish the message named it: «Equidistancia limitada a
                # 0.02 m» in the middle of a push, with no reason given. A
                # correct limit that reads as a broken tool is a bug of its own
                # (Marco, 2026-09-10: «quiero hacer push para abajo y no me
                # deja»; the limit was 2 cm of material under one corner).
                viewport.flash_status(
                    tr("Push limited to {value} — deeper would leave the "
                       "solid", value=fmt_len(self._limit_in)), 5000)

    @staticmethod
    def _cap_loop_positions(face) -> list[QVector3D]:
        """Every boundary position of ``face`` — outer loop *and* hole rims.
        The prism-translate path moves all of them (a hole rim left behind
        warps the cap into a non-planar holed face), and reference inference
        excludes all of them from snapping."""
        pts = [QVector3D(v) for v in face.vertices]
        for hole in face.holes:
            pts.extend(QVector3D(v) for v in hole)
        return pts

    @classmethod
    def _outward_normal(cls, face, mesh) -> QVector3D:
        """The normal this drag signs ``extrusion`` along: ``face``'s own, or
        the surface's when the face disagrees with it and nothing else can say.

        The tool's whole sign convention rests on the base pointing OUTWARD —
        the recess rule reads "negative = into the material, so hide the base
        and show an opening". ``orient_outward`` establishes that at drag
        start and is the authority whenever it HAS a volume to test parity
        against, so a closed mesh is left to it untouched. A flat sheet has no
        outward at all, and is left alone too.

        What is left is the OPEN shell, where orient_outward has nothing to
        count and leaves every winding as the draw made it. A face that came
        out wound the other way then points INTO the model, a drag into the
        wall reads as positive, the base is never hidden, and the outer face
        stands there covering the pocket forming behind it: pushing a door
        into Marco's cube looked like nothing was happening while the push
        itself worked (cubo.igz, 2026-08-27 — a door disagreeing with all
        five of its coplanar neighbours).

        There, the coplanar faces across the boundary ARE the surface this
        face was cut out of, so they are what it has to agree with; a majority
        settles it, and no coplanar neighbour at all leaves the normal alone.
        """
        if is_closed(mesh) or _mesh_is_flat(mesh):
            return face.normal()
        return cls._surface_normal(face, mesh)

    @staticmethod
    def _surface_normal(face, mesh=None) -> QVector3D:
        """``face``'s unit normal, turned to agree with the majority of its
        coplanar neighbours: the surface it was cut out of
        (``pushscan::scan_surface_normal``)."""
        return push_scans.surface_normal(mesh if mesh is not None else face._m, face)

    def _classify_base(self, scene) -> tuple[bool, bool]:
        """Classify the base face for previewing.

        Returns ``(attached, prism_cap)``:
        - ``attached`` — every edge is shared (embedded in a surface/solid), so
          an inner face pushed in is a recess (hidden so the pocket shows).
        - ``prism_cap`` — every edge backs onto a *perpendicular* wall, so the
          push is a clean prism extend/shrink and can be previewed by live
          translation (walls deform with the cap, clean both ways).

        Hole rims count as edges of the cap too: a hole edge backing onto a
        coplanar inner panel (a rectangle drawn on a wall) breaks prism_cap —
        translating the cap would leave the panel and rim behind. A hole whose
        rim backs onto perpendicular swept walls (a window tube through the
        slab) keeps the clean translate. A face with a corner on the rim
        that is not such a wall (the next room's corner on a wall's edge)
        would be skewed by the translate: no prism cap then, unless the
        corner only slides along a straight edge running with the push.
        The rule is Rust's (``pushscan::scan_classify_base``).
        """
        attached, prism_cap = push_scans.classify_base(scene.mesh, self.base_face)
        return attached, prism_cap

    def _commit(self, viewport) -> None:
        if self.base_face is not None and not self._ensure_store(viewport.scene):
            # A drag armed without a click (a script, a test helper) meets
            # its Python mesh here, still outside the History step.
            viewport.flash_status(tr("Push refused: the geometry kernel could not take "
                                     "this mesh (left it unchanged)"), 6000)
            self._reset()
            return
        viewport.set_hover(None)  # the hovered face is about to be replaced
        self._light_faces = []
        self._light_wires = []
        viewport.set_suppressed_faces(set())
        # A held stage showing this very distance is kept as the result;
        # otherwise drop the live preview and redo it for real.
        cmd = self._staged_command(viewport)
        if cmd is None:
            self._revert_preview(viewport)
        if self.base_face is None or abs(self.extrusion) < _MIN_EXTRUDE:
            self._reset()
            self._spend_keep_mode(viewport)
            viewport.update()
            return
        # One snapshot wraps the edit *and* the watertight stitch: undo is exact,
        # and it is the identical mutation the live preview just showed. A push
        # aimed at a group snapshots that group's mesh instead.
        self._topped_out = False
        self._multi_missed = 0
        if cmd is None:
            cmd = SnapshotMutation(
                self._mutate_many if self._extra_faces
                else self._mutate_or_top_out,
                mesh=self._group.mesh if self._group is not None else None)
        viewport.history.execute(cmd)
        # The retype window's payload, captured BEFORE _reset clears the drag
        # state (the sign is the direction actually committed — the reduced
        # distance on a top-out). Armed only where a push landed: a refusal
        # left the mesh untouched, so there is nothing to retype.
        last = None
        if self._topped_out:
            viewport.flash_status(tr(
                "Push stopped at {d}: going further would break the "
                "solid", d=fmt_len(abs(self.extrusion))), 4000)
            type(self).last_distance = self.extrusion
            last = self._retype_state(cmd)
        elif self._refused and self._store_refusal is not None:
            viewport.flash_status(
                tr("Push refused: the geometry kernel could not finish it "
                   "(left it unchanged; a report was saved)"), 6000)
        elif self._refused:
            # The guard rolled the push back to keep the solid watertight; tell
            # the user so the no-op isn't silent — long timeout, it's easy to
            # miss and the tool otherwise looks broken.
            viewport.flash_status(
                tr("Push refused: would break the solid (left it unchanged)"),
                6000)
        else:
            type(self).last_distance = self.extrusion  # double-click repeats it
            last = self._retype_state(cmd)
            if self._multi_missed:
                total = len(self._extra_faces) + 1
                viewport.flash_status(tr(
                    "Pushed {done} of {total} faces — the others would "
                    "break the solid", done=total - self._multi_missed,
                    total=total), 6000)
        self._reset()
        self._spend_keep_mode(viewport)
        self._last = last             # arms the retype window, if one opened
        viewport.update()

    def _retype_state(self, cmd) -> dict:
        """The ``_last`` payload: everything a hot retype (on_value, tool
        idle) needs to replay the push _commit is about to finish — the drag
        state _reset clears next line, plus the command to beat on the undo
        stack's top. See the ``_last`` declaration for the window contract."""
        return {
            "cmd": cmd,
            "face": self.base_face,
            "group": self._group,
            "keep_base": self._keep_base,
            "prism_cap": self._prism_cap,
            # _normal can be None on a directly-driven commit (tests); the
            # replay falls back to the face's own, exactly as the original.
            "normal": (None if self._normal is None
                       else QVector3D(self._normal)),
            "cap_positions": list(self._cap_positions),
            "pre_oriented": self._drag_pre_oriented,
            "limit_in": self._limit_in,
            "sign": 1.0 if self.extrusion >= 0.0 else -1.0,
            "extras": list(self._extra_faces),
            "extra_normals": list(self._extra_normals),
        }

    #: The drag state that belongs to ONE base face: _mutate_many swaps it
    #: per face and hands the base face's back.
    _FACE_STATE = ("base_face", "_normal", "_anchor", "_attached",
                   "_prism_cap", "_cap_positions", "_limit_in",
                   "_drag_pre_oriented", "_refused", "_topped_out",
                   "extrusion")

    def _mutate_many(self, scene):
        from ingetrazo_core import push_secondary_faces
        wanted = self.extrusion
        self._mutate_or_top_out(scene)
        lead = {k: getattr(self, k) for k in self._FACE_STATE}
        mesh = scene.mesh
        shapes = [_face_shape(f) for f in self._extra_faces]

        def prepare(index):
            face = self._extra_faces[index]
            if face not in mesh.faces:
                face = next((f for f in mesh.faces if _face_shape(f) == shapes[index]), None)
            if face is None:
                return None
            orient_outward(mesh, only=(face,))
            self.base_face = face
            self._drag_pre_oriented = True
            self._anchor = face.centroid()
            self._normal = self._outward_normal(face, mesh)
            self._attached, self._prism_cap = self._classify_base(scene)
            self._cap_positions = self._cap_loop_positions(face)
            self.extrusion = wanted
            self._compute_inward_limit(scene)
            self._clamp_extrusion()
            return self.extrusion

        def apply():
            self._mutate_or_top_out(scene)
            return self._refused, self._topped_out

        try:
            refused, missed = push_secondary_faces(mesh, len(shapes), wanted, _MIN_EXTRUDE,
                                                   self._refused, prepare, apply)
        finally:
            for name, value in lead.items():
                setattr(self, name, value)
        self._refused, self._multi_missed = refused, missed
        self._topped_out = False
        self.extrusion = wanted

    #: How many halvings the commit tries when the guard refuses the distance
    #: the user dragged to. Each costs a full pipeline run, so this buys a
    #: reachable height for about a second in the worst case — only ever spent
    #: on a push that would otherwise do nothing at all.
    _CLAMP_TRIES = 6

    def _mutate_or_top_out(self, scene):
        from ingetrazo_core import push_top_out

        def attempt(distance):
            self.extrusion = distance
            self._mutate(scene)
            if self._store_refusal is not None:
                # The kernel, not the guard: a shorter push is not tried.
                raise _Abandoned
            return self.extrusion, self._refused

        wanted = self.extrusion
        try:
            self.extrusion, self._refused, partial = push_top_out(
                self._target_mesh(scene), attempt, self.extrusion, _MIN_EXTRUDE, self._CLAMP_TRIES)
        except _Abandoned:
            self.extrusion, self._refused, partial = wanted, True, False
        if partial:
            self._topped_out = True

    #: Why the store refused the last push (None: it did not), for the
    #: status; set by _mutate.
    _store_refusal: str | None = None

    def _mutate(self, scene, preview: bool = False) -> None:
        """:meth:`_mutate_guarded` with the store strict: a store step that
        refuses refuses the push. The mesh is put back as it was, the push
        reads as refused and a repro is written (:func:`_write_repro`)."""
        from core.store_mesh import StoreMesh, StoreRefusal, strict
        self._store_refusal = None
        mesh = self._target_mesh(scene)
        snap = mesh.capture_state() if type(mesh) is StoreMesh else None
        try:
            with strict():
                self._mutate_guarded(scene, preview)
        except StoreRefusal as e:
            if snap is None:
                raise
            mesh.restore_state(snap)
            self._refused = True
            self._store_refusal = str(e)
            _write_repro(mesh, self, str(e))

    def _mutate_guarded(self, scene, preview: bool = False) -> None:
        """Apply the push and stitch watertight, with a **BIM-grade refusal
        guard**: a push on a closed solid must leave a closed solid (or collapse
        it flat). If the operation would leave the mesh cracked — an ill-defined
        push *through* an interior partition, a hole rim detached from a column
        anchored to it, a degenerate collapse — the result is **refused**: the
        solid is restored untouched rather than committing a non-watertight mesh
        that would poison volume/area takeoff and IFC/STL export. A surface
        modeler commits the mess; a solid modeler feeding a quantity engine
        must not.

        The guard only fires on a mesh that *was* a valid closed solid; a flat
        sheet (a recess in a surface, the first flat→solid extrude) legitimately
        opens, so it is never guarded."""
        self._refused = False
        target = self._target_scene(scene)
        mesh = target.mesh
        before = mesh._store.snapshot()
        # Pushing a tagged face EXTENDS its BIM object: the walls and cap the
        # extrusion creates join the base's tag (thickening a tagged slab, or
        # raising a wall from a rect the active class stamped, yields a fully
        # tagged solid). Capture before the mutation — the base is consumed.
        base_tag = None
        base_paint = None
        if self.base_face is not None:
            t = self.base_face.attrs.get("ifc")
            if t:
                base_tag = dict(t)
            base_paint = _paint_of(self.base_face)
        from core.store_mesh import _store_of
        self._refused = guarded_push(
            mesh, lambda: self._mutate_inner(scene, preview),
            abs(self.extrusion) >= _MIN_EXTRUDE, is_closed, _mesh_is_flat, _store_of)
        if self._refused:
            return

        shift = (self._normal * self.extrusion).toTuple() if self._normal is not None else None
        key = self.base_face.key if self.base_face is not None else None
        fresh = mesh.finish_push(before, key, shift, self._CURVE_FACET_COS,
                                 bool(base_tag is not None or base_paint))
        for face in fresh:
            if base_tag is not None and not face.attrs.get("ifc"):
                face.attrs["ifc"] = dict(base_tag)
            if base_paint and not any(k in face.attrs for k in PAINT_KEYS):
                face.attrs.update(base_paint)

    # Shared with the drag preview. A swept circle softens at a shallow
    # dihedral while a hexagon or octagon keeps its corners.
    _CURVE_FACET_COS = 0.85

    def _mutate_inner(self, scene, preview: bool = False) -> None:
        """Apply the push to the target mesh (the scene's, or the locked
        group's) and stitch it watertight. Shared by the committed edit
        (wrapped in :class:`SnapshotMutation`) and the live preview, so the
        drag renders exactly what will commit."""
        scene = self._target_scene(scene)
        face = self.base_face
        d = self.extrusion

        # A prism cap is exactly a translation of the cap along its normal; the
        # walls follow through shared vertices. Then repair connectivity
        # (welds, T-junctions, collinear) — no coplanar-merge, nothing new is
        # added. Ctrl ("push/pull a copy") must not translate: it stacks a new
        # segment, so it takes the extrude path below with the base kept.
        if translate_prism_cap(self._prism_cap, self._keep_base,
                               lambda: self._cap_passes_a_wall_vertex(scene, face, d)):
            normal = self._normal if self._normal is not None else face.normal()
            scene.mesh.push_translate(self._cap_positions, tuple(n * d for n in normal.toTuple()),
                                      removing=d < 0, preview=preview)
            scene.version += 1
            return

        scene.mesh.push_extrude(scene, face, d, self._normal.toTuple() if self._normal is not None else None,
                                self._keep_base, self._drag_pre_oriented, preview)

    def _cap_passes_a_wall_vertex(self, scene, face, d: float) -> bool:
        """Whether shrinking the prism cap by ``d`` carries it past a vertex
        partway up one of its walls — the next room's corner sitting on the
        wall. The translate leaves that vertex where it is, so the wall would
        fold into a spike around it (or a zero-area sliver at a full
        collapse); the extrude path cuts the wall instead."""
        if d >= 0:
            return False
        normal = self._normal if self._normal is not None else face.normal()
        return push_scans.cap_passes_wall(scene.mesh, face, self._cap_positions, normal, d)

    def _reset(self) -> None:
        self.hovered_face = None
        self.base_face = None
        self.extrusion = 0.0
        self.dragging = False
        self._attached = False
        self._prism_cap = False
        self._prism_applied = 0.0
        self._prism_verts = []
        self._keep_base = False
        self._limit_in = None
        self._group = None
        self._hover_group = None
        self._selected_arm = False
        self._extra_faces = []
        self._extra_normals = []
        self._anchor = None
        self._grab = None
        self._normal = None
        self._cap_positions = []
        self._stages = PushStages()
        self._plane_levels = []
        self._drag_pre_oriented = False
        self._inference_point = None
        self._inference_kind = None
        self._infer_cache = None       # per-drag projected-vertex candidates
        self._refused = False
        self._light_faces = []
        self._light_wires = []
        self._last = None             # the retype window closes with the drag

    def _spend_keep_mode(self, viewport) -> None:
        """The Ctrl mode is spent by the push it armed, like Copy in Move
        and Rotate: the bar stops advertising it the moment the push lands
        (or Esc drops it), so the modifier never leaks into the next face."""
        if not self._keep_mode:
            return
        self._keep_mode = False
        self._keep_base = False
        self.refresh_hint(viewport)
