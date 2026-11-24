# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Push/Pull as an operation: classify the face, find how far it may go,
and commit the extrusion, the push-through or the hole. The one
implementation behind the Push/Pull tool and the protocol's ``push_pull``
(moved out of ``tools/pushpull.py``, docs/rust-tools.md §18)."""
from __future__ import annotations
from core.element_set import ElementSet

from ingetrazo_core import Vec3 as QVector3D

from core.geometry import Face
from core.i18n import tr
from core.mesh import _STITCH_TOL, PAINT_KEYS, Mesh, turn_face_over
from core.history import (
    AddEdgeCommand,
    AddFaceCommand,
    DeleteFaceCommand,
    PruneOrphanEdgesCommand,
    SnapshotMutation,
    run_stitch,
    subtract_loop_from_face,
    translate_points,
)
from core.arrangement import _interior_point, _point_in_polygon, plane_basis
from core.orient import is_closed, orient_outward
from core.cap_rebuild import (
    RebuildCache,
    _canon_loop,
    apply_rebuild,
    crack_planes,
    edges_along,
    seam_planes,
)
from core.topology import (
    _faces_coplanar,
    _key,
    _mesh_is_flat,
    classify_push_edge,
    heal_overlapping_faces,
    loop_inside_face,
    _segment_on_face_boundary,
    refine_loop_with_points,
)
from core.units import fmt_len
from core import push_scans
from ingetrazo_core import guarded_push, push_region, push_through_face, translate_prism_cap


def _project_loop_2d(loop, normal):
    u, v = plane_basis(normal)
    return [(QVector3D.dotProduct(p, u), QVector3D.dotProduct(p, v)) for p in loop]


def _loops_overlap_2d(a_xy, b_xy) -> bool:
    """Whether two loops projected to the same plane overlap laterally: a
    representative interior point of one inside the other, or any vertex
    strictly inside. Good for the push-clamp candidates (a far cap under the
    whole loop, a floor under a corner notch); exotic offset-cross overlaps
    with no contained vertex are not the clamp's concern. Holes are ignored."""
    if _point_in_polygon(_interior_point(a_xy), b_xy):
        return True
    if _point_in_polygon(_interior_point(b_xy), a_xy):
        return True
    return any(_point_in_polygon(p, b_xy) for p in a_xy) or \
        any(_point_in_polygon(p, a_xy) for p in b_xy)


class _PushBand:
    """The connecting edges and faces of one pushed ring, inside the push transaction."""

    def __init__(self, base, top, *, rim=False, rails=False, walls=False, cap=False):
        self.base, self.top = base, top
        self.parts = rim, rails, walls, cap

    def do(self, scene):
        from core.store_mesh import StoreMesh
        from ingetrazo_core import MeshStore
        mesh = scene.mesh
        if type(mesh) is StoreMesh:
            dirty, steps = mesh._store.push_band([mesh._input_point(p) for p in self.base],
                                                 [mesh._input_point(p) for p in self.top], *self.parts)
            if dirty:
                mesh._dirty(dirty)
            scene.version += steps
            return
        edges, faces = MeshStore.object_push_band([p.toTuple() for p in self.base],
                                                  [p.toTuple() for p in self.top], *self.parts)
        for a, b in edges:
            AddEdgeCommand(QVector3D(*a), QVector3D(*b)).do(scene)
        for ring in faces:
            face = mesh.add_face([QVector3D(*p) for p in ring])
            face.interior = False
            scene.version += 1


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


def _segment_faces(mesh, a, b, faces):
    """The faces ``classify_push_edge(face, a, b, faces)`` /
    ``_segment_on_face_boundary(a, b, g)`` can find carrying ``a``–``b``:
    the native candidates near both points (list order, a superset), or
    ``faces`` itself with the scans off (``core.push_scans``)."""
    got = push_scans.faces_on_segment(mesh, a, b)
    return faces if got is None else got


def _classify_edge(face, a, b, mesh, faces, memo):
    """``classify_push_edge`` of ``a``–``b``: over the native candidates
    near the segment when the scans are on (:func:`_segment_faces`), or
    over ``faces`` with ``memo`` — the coplanar split remembered across
    the edges of one face, valid only while the list asked about is the
    same one each time."""
    got = push_scans.faces_on_segment(mesh, a, b)
    if got is None:
        return classify_push_edge(face, a, b, faces, memo)
    return classify_push_edge(face, a, b, got)


def _refine_candidates(mesh, loops) -> list:
    """The positions ``refine_loop_with_points(loop, points)`` can insert
    into ``loops``' edges: every vertex inside the box it tests around an
    edge (``2·tol + 1e-6·|coords|``, grown a hair), in list order — or every
    vertex with the scans off. The same points per edge, in the same order,
    so the same refined loops."""
    from core.topology import _SPLIT_TOLERANCE as tol
    boxes = []
    for loop in loops:
        n = len(loop)
        for i in range(n):
            a, b = loop[i], loop[(i + 1) % n]
            big = max(abs(a.x()), abs(a.y()), abs(a.z()),
                      abs(b.x()), abs(b.y()), abs(b.z()))
            g = 2.0 * tol + 1e-6 * big + 1e-9 * (1.0 + big)
            boxes.append((min(a.x(), b.x()) - g, min(a.y(), b.y()) - g,
                          min(a.z(), b.z()) - g, max(a.x(), b.x()) + g,
                          max(a.y(), b.y()) + g, max(a.z(), b.z()) + g))
    got = push_scans.vertices_in_boxes(mesh, boxes) if boxes else []
    if got is None:
        return [v.position for v in mesh.vertices]
    return [v.position for v in got]


def _made_since(mesh, snap, kind: str):
    """The elements of ``kind`` a store-backed ``mesh`` holds and its
    snapshot ``snap`` (``capture_state()``) does not, in list order — what
    ``[x for x in mesh.<kind>s if x not in snap]`` finds — or ``None`` (a
    Python mesh or snapshot, or the scans off)."""
    from core.store_mesh import StoreState
    if not isinstance(snap, StoreState):
        return None
    return push_scans.made_since(mesh, snap.snap, kind)


class _FacesBefore:
    """The faces before one plane rebuild, for the two questions the
    fixpoint asks after it: which faces are new (``set(mesh.faces) -
    before_faces``, in that set's iteration order, which ``fresh`` and so
    the next round's planes inherit) and whether a new face's shape is the
    shape of a face from before that is not fresh (``kept``).

    The first needed a set of every face per plane (53 ms of views on the
    ride's mesh) and the second the shape of every face but the fresh ones
    (seconds). A store-backed mesh answers the first by key
    (``push_scans.set_minus``: the same table, the same order); the second
    is asked of the faces from before near the new face only — an equal
    shape has its corners in the same 1e-4 cells, so its box is within a
    cell of the new face's — through the same ``_face_shape`` of the same
    elements (a store keeps a removed face's rings readable)."""

    __slots__ = ("keys", "faces", "scan", "_python")

    def __init__(self, mesh) -> None:
        from core import scan_index
        self.keys = push_scans.live_keys(mesh, "face")
        self.faces = set(mesh.faces) if self.keys is None else None
        # A store scan of the state before (cached per revision; it keeps
        # that state's faces by key). A Python mesh's faces are scanned
        # only if a rebuild changed something: they keep their loops, and
        # a rebuild moves no vertex.
        self.scan = scan_index.scan(mesh) if self.keys is not None else None
        self._python = self.keys is None and scan_index.enabled()

    def new_faces(self, mesh):
        if self.keys is not None:
            now = push_scans.live_keys(mesh, "face")
            if now is not None:
                got = push_scans.set_minus(mesh, "face", now, set(self.keys))
                if got is not None:
                    return got
            self.faces = set(push_scans.views(mesh, "face", self.keys))
        return set(mesh.faces) - self.faces

    def kept(self, face, fresh) -> bool:
        shape = _face_shape(face)
        if self.scan is None and self._python:
            from core import _native, scan_index
            self._python = False
            self.scan = scan_index.Scan.of_faces(_native.native,
                                                 list(self.faces))
        if self.scan is None:
            if self.faces is None:
                self.faces = set(push_scans.views(
                    face._m, "face", self.keys))
            return any(_face_shape(g) == shape for g in self.faces
                       if g not in fresh)
        pts = face.vertices
        m = 2e-4 + 1e-6 * max((max(abs(p.x()), abs(p.y()), abs(p.z()))
                               for p in pts), default=0.0)
        lo = (min(p.x() for p in pts) - m, min(p.y() for p in pts) - m,
              min(p.z() for p in pts) - m)
        hi = (max(p.x() for p in pts) + m, max(p.y() for p in pts) + m,
              max(p.z() for p in pts) + m)
        near = self.scan.faces(self.scan.faces_meeting((lo, hi)))
        return any(_face_shape(g) == shape for g in near if g not in fresh)


def _component_is_closed(mesh, face) -> bool:
    """Whether ``face``'s edge-connected component is watertight — every edge
    reachable from it borders at least two faces. An open sheet (a flat
    drawing) reaches a boundary edge and reads ``False`` even when a closed
    solid exists elsewhere in the same mesh."""
    seen = {face}
    stack = [face]
    while stack:
        f = stack.pop()
        for lp in (f.loop, *f.hole_loops):
            n = len(lp)
            for i in range(n):
                e = mesh.find_edge(lp[i], lp[(i + 1) % n])
                if e is None:
                    continue
                if len(e.faces) < 2:
                    return False
                for g in e.faces:
                    if g not in seen:
                        seen.add(g)
                        stack.append(g)
    return True


def _swept_by_push(neighbour: Face, push_normal: QVector3D) -> bool:
    """Whether ``neighbour`` is *swept* by a push along ``push_normal`` — it has
    an edge parallel to the push, so translating its shared edge slides the wall
    along its own length (a prism wall getting taller). A cap perpendicular to
    the prism axis (a triangular floor/top) has no such edge, so pushing one of
    *its* edges out would shear it instead — that case must add a coplanar strip
    (a box bump), not translate/extend."""
    n = push_normal.normalized()
    verts = neighbour.vertices
    m = len(verts)
    for i in range(m):
        e = verts[(i + 1) % m] - verts[i]
        if e.length() > 1e-9 and abs(QVector3D.dotProduct(e.normalized(), n)) > 0.999:
            return True
    return False


def _corner_dragged(face: Face, rim: set, push_normal: QVector3D) -> bool:
    """Whether a prism-cap translate moving the ``rim`` positions along
    ``push_normal`` would bend ``face`` at one of them — any of its vertices
    on the rim that isn't a straight-through point of an edge running with
    the push."""
    n = push_normal.normalized()
    for loop in [face.vertices, *face.holes]:
        m = len(loop)
        for i, v in enumerate(loop):
            if _key(v) not in rim:
                continue
            a, b = loop[i - 1] - v, loop[(i + 1) % m] - v
            if (a.length() < 1e-9 or b.length() < 1e-9
                    or abs(QVector3D.dotProduct(a.normalized(), n)) < 0.999
                    or abs(QVector3D.dotProduct(b.normalized(), n)) < 0.999):
                return True
    return False


# The smallest extrusion that still moves geometry: anything below the mesh's
# weld resolution (1e-4, ~0.1 mm) would weld the top loop onto the base loop
# vertex-by-vertex and degenerate the wall quads. Treated as a no-op push.
_MIN_EXTRUDE = 2e-4

#: Below this many pixels between press and release, the "drag" was a
#: jittery click, not a stroke: the release belongs to the click-click
#: rhythm and must not commit. Same release-vs-click disambiguation as
#: Follow Me (tools/followme.py), same threshold.
_DRAG_PX = 6.0




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


def _seam_planes_along(mesh, segments) -> list:
    """``(origin, normal)`` of each plane where an edge along ``segments``
    parts two coplanar faces facing the same way with the same paint."""
    out = []
    for e in edges_along(mesh, segments):
        if len(e.faces) != 2:
            continue
        fa, fb = e.faces
        if (fa.interior or fb.interior
                or (fa.attrs or {}) != (fb.attrs or {})
                or QVector3D.dotProduct(fa.normal().normalized(),
                                        fb.normal().normalized()) < 0.9999):
            continue
        out.append((fa.centroid(), fa.normal()))
    return out


def _faces_window(faces):
    """The box around ``faces`` grown by 1 cm (plus a relative term far from
    the origin), or ``None`` for none — every crack then, as before."""
    xs, ys, zs = [], [], []
    for f in faces:
        for lp in (f.loop, *f.hole_loops):
            for v in lp:
                p = v.position
                xs.append(p.x()); ys.append(p.y()); zs.append(p.z())
    if not xs:
        return None
    lo = (min(xs), min(ys), min(zs))
    hi = (max(xs), max(ys), max(zs))
    m = 0.01 + 1e-6 * max(abs(c) for c in lo + hi)
    return (lo[0] - m, lo[1] - m, lo[2] - m, hi[0] + m, hi[1] + m, hi[2] + m)



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
        # face stays put on THIS push, which is then spent (the classic
        # one-shot modifier, like Copy in Move). `_ctrl_used` remembers that
        # Ctrl was held when the gesture began, so its release does not
        # toggle the mode on the way out.
        self._keep_mode: bool = False
        self._ctrl_used: bool = False
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
        # The clean model under a STAGED preview (see _show_staged_preview):
        # captured before the drag's first stage and kept until the drag ends
        # — restored whenever a staged model goes back to clean (a restage,
        # the overlay, Esc, a tool switch, a pipeline commit), and the undo
        # state of a commit that keeps the stage. The selection rides along:
        # the stage deletes the base face, which drops it from the selection.
        self._preview_snapshot: dict | None = None
        self._preview_selection: set = set()
        # How much of ``extrusion`` the prism preview has already applied to
        # the model, so a frame moves only the delta and the revert knows the
        # total to undo, and the very Vertex OBJECTS it moves.
        self._prism_applied: float = 0.0
        self._prism_verts: list = []
        # ---- Staged preview -------------------------------------------------
        # The distance the staged result was built at (None = the model is
        # NOT staged: the one indicator _revert_preview restores on), where
        # its cap currently sits, the new cap-plane Vertex OBJECTS a frame
        # slides (with their staged positions and slide rates — see
        # _slide_rate), and the open interval of
        # distances the slide stays exact in. Kept apart from the prism fields:
        # the per-frame _revert_prism_preview would otherwise undo them.
        self._stage_d: float | None = None
        self._stage_applied: float = 0.0
        self._stage_verts: list = []
        self._stage_pos: list = []
        self._stage_lo: float = 0.0
        self._stage_hi: float = 0.0
        # The faces/edges the stage created: the distance inference skips
        # them, or the drag would lock onto its own moving cap.
        self._stage_new: set = set()
        # Stages left behind when the cursor leaves their interval, at most
        # one per side of the base plane ({sign: record}, see _park_stage), so
        # dragging back and forth restores one instead of re-running the
        # pipeline; and the record the current stage came from (None = not
        # parked yet).
        self._stage_cache: dict = {}
        self._stage_rec: dict | None = None
        # ``(sign, abs(d))`` of the last refused or failed stage: that
        # direction shows the overlay and retries only once the distance has
        # doubled, so a refusal never re-runs the pipeline per frame, and a
        # tiny first frame refused by float noise does not cost the whole
        # direction its live result.
        self._stage_refusal: tuple | None = None
        # A stage slower than _SLOW_STAGE_S: no more stages this drag.
        self._stage_slow: bool = False
        # Levels (along the normal) of the parallel faces over/under the base,
        # collected at drag start: crossing one changes the topology.
        self._plane_levels: list = []
        # ``(id(mesh), mesh._mut_serial)`` after the tool's own last edit while
        # a snapshot is held: anything else touching the mesh mid-drag (File >
        # New, a redo that bypassed on_redo) makes the snapshots stale, and
        # restoring one would resurrect the model it replaced.
        self._stage_serial: tuple | None = None
        # The model point the distance inference is currently locked onto (a
        # corner or a face hit), drawn as a green marker by the viewport overlay.
        self._inference_point: QVector3D | None = None
        self._inference_kind: str | None = None
        # Set when the last _mutate refused the push to keep the solid valid
        # (the BIM-grade guard); _commit surfaces it in the status bar.
        self._refused: bool = False
        self._topped_out: bool = False
        # One "too small to commit" hint per drag: a click that does nothing
        # with no word reads as a broken tool, but repeating it on every tiny
        # click is its own annoyance. Cleared in _reset, so each drag gets one.
        self._tiny_hint_shown: bool = False
        # ---- Press-drag-release --------------------------------------------
        # Where the press landed and where the cursor is now (screen px):
        # the release compares the two to tell a stroke from a jittery click.
        # Drags armed without a real press — the hot-retype replay, the test
        # harness — leave both None, and on_release copes by doing nothing.
        self._press_screen: tuple | None = None
        self._last_screen: tuple | None = None
        # ---- Drag preview -----------------------------------------------
        # A free face and a Ctrl-copy show the naive sweep as an overlay — cap
        # plus wall quads, nothing touched in the mesh. An attached face is
        # STAGED instead (_show_staged_preview): the real pipeline runs once
        # and frames only slide its cap. Running the pipeline per mouse-move
        # cost ~0.3 s a frame on an imported barbecue.
        self._light_faces: list = []
        # The sweep's rings, ``[(base_ring, moved_ring), …]`` (outer first,
        # then holes) — the wireframe is drawn straight off them.
        self._light_rings: list = []
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
        step = (tuple(n * delta for n in self._normal.toTuple()) if "_store" in mesh.__dict__
                else self._normal * delta)
        for v in self._prism_verts:
            mesh.move_vertex(v, step)
        # Booked as the TARGET, not as a running sum: adding deltas frame by
        # frame drifts, and the revert has to undo exactly what it applied.
        self._prism_applied = applied
        viewport.scene.version += 1

    def _revert_prism_preview(self, viewport) -> None:
        """Put a translated cap back where it started."""
        if self._prism_applied:
            self._move_cap(viewport, -self._prism_applied, 0.0)

    def _slide_stage(self, viewport, d: float) -> None:
        """Slide the stage's moving vertices to distance ``d``. Placed at
        staged position + rate × offset, not moved by deltas: float32 deltas
        drift frame by frame (place_vertex, issue #163)."""
        if not self._stage_verts or abs(d - self._stage_applied) <= 1e-12:
            return
        mesh = self._target_mesh(viewport.scene)
        off = d - self._stage_d
        for v, (p, rate) in zip(self._stage_verts, self._stage_pos):
            position = (tuple(a + b * off for a, b in zip(p, rate.toTuple()))
                        if isinstance(p, tuple) else p + rate * off)
            mesh.place_vertex(v, position)
        self._stage_applied = d
        self._stage_serial = (id(mesh), mesh._mut_serial)
        viewport.scene.version += 1

    @staticmethod
    def _interval_holds(d: float, stage_d, lo: float, hi: float) -> bool:
        """Whether a stage built at ``stage_d`` with interval ``(lo, hi)``
        shows the commit at distance ``d``: its own distance, or strictly
        inside — kept a weld tolerance clear of the levels, where the commit
        welds the cap onto them."""
        if stage_d is None:
            return False
        if abs(d - stage_d) <= 1e-9:
            return True
        return lo + _STITCH_TOL < d < hi - _STITCH_TOL

    def _stage_holds(self, d: float) -> bool:
        return self._interval_holds(d, self._stage_d, self._stage_lo,
                                    self._stage_hi)

    def _restore_clean(self, viewport) -> None:
        """Back to the clean model the stages are built on, and its
        selection. The current stage is dropped; a parked copy survives."""
        scene = viewport.scene
        mesh = self._target_mesh(scene)
        mesh.restore_state(self._preview_snapshot)
        scene.selection.clear()
        scene.selection.update(self._preview_selection)
        self._clear_stage()
        self._stage_serial = (id(mesh), mesh._mut_serial)
        scene.version += 1

    def _clear_stage(self) -> None:
        """Forget the current stage (the model's state is the caller's)."""
        self._stage_d = None
        self._stage_applied = 0.0
        self._stage_verts = []
        self._stage_pos = []
        self._stage_new = set()
        self._stage_rec = None

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
                or not self._stage_holds(self.extrusion)):
            return None
        self._slide_stage(viewport, self.extrusion)
        mesh = self._target_mesh(viewport.scene)
        cmd = SnapshotMutation.from_states(
            self._preview_snapshot, mesh.capture_state(),
            mesh=self._group.mesh if self._group is not None else None)
        # The command owns the snapshots now: nothing is restored.
        self._preview_snapshot = None
        self._preview_selection = set()
        self._stage_cache = {}
        self._clear_stage()
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
        if self._stage_serial == (id(mesh), mesh._mut_serial):
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
            self._restore_clean(viewport)

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
        return cls._surface_normal(face)

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
        slab) keeps the clean translate.
        """
        faces = scene.faces
        kinds = []
        memo: dict = {}
        for loop in [self.base_face.vertices] + self.base_face.holes:
            n = len(loop)
            kinds.extend(
                _classify_edge(self.base_face, loop[i], loop[(i + 1) % n],
                               scene.mesh, faces, memo)
                for i in range(n)
            )
        normal = self.base_face.normal()
        attached = all(kind != "free" for kind, _ in kinds)
        # A clean prism-cap translate only applies when every backing wall is
        # *swept* by the push (has an edge along the normal). Otherwise the push
        # is into a cap that would shear — handle that via the extrude path so it
        # adds a box bump instead of dragging the whole cross-section.
        prism_cap = bool(kinds) and all(
            kind == "perp" and nb is not None and _swept_by_push(nb, normal)
            for kind, nb in kinds
        )
        if prism_cap:
            # The translate moves every face carrying a rim position. A face
            # that only touches the rim at a vertex — the next room's corner
            # landing on a wall's edge — shares no edge with the cap, so it
            # isn't a wall the push stretches: dragging its corner skews it.
            # Unless the position only slides along a straight edge of it that
            # runs with the push (a T-point on the host wall's top edge).
            walls = {nb.key for _, nb in kinds}
            rim_pts = [p for loop in [self.base_face.vertices]
                       + self.base_face.holes for p in loop]
            rim = {_key(p) for p in rim_pts}
            # Only a face with a corner on the rim can be dragged by it.
            near = push_scans.faces_near_points_rings(
                scene.mesh, [_key(p) for p in rim_pts])
            prism_cap = not any(
                _corner_dragged(g, rim, normal)
                for g in (faces if near is None else near)
                if g != self.base_face and g.key not in walls)
        return attached, prism_cap

    def _commit(self, viewport) -> None:
        viewport.set_hover(None)  # the hovered face is about to be replaced
        self._light_faces = []
        self._light_rings = []
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
            return self.extrusion, self._refused

        self.extrusion, self._refused, partial = push_top_out(
            self._target_mesh(scene), attempt, self.extrusion, _MIN_EXTRUDE, self._CLAMP_TRIES)
        if partial:
            self._topped_out = True

    def _mutate(self, scene, preview: bool = False) -> None:
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
        # The edges there before, to soften only the push's own: by key on
        # a store-backed mesh (a set of 534k views took 0.65 s on the ride).
        before_mark = push_scans.mark(mesh)
        before_edges = set(mesh.edges) if before_mark is None else None
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
        before_verts = ({v.key for v in mesh.vertices}
                        if (base_tag is not None or base_paint)
                        and before_mark is None else None)
        curve_runs = (self._curve_runs(mesh, self.base_face)
                      if self.base_face is not None else [])
        from core.store_mesh import _store_of
        self._refused = guarded_push(
            mesh, lambda: self._mutate_inner(scene, preview),
            abs(self.extrusion) >= _MIN_EXTRUDE, is_closed, _mesh_is_flat, _store_of)
        if self._refused:
            return

        self._soften_curve_facets(mesh, before_edges, before_mark)
        self._tag_moved_curves(mesh, curve_runs)
        if base_tag is not None or base_paint:
            # Only faces carrying at least one vertex this push created — the
            # extrusion's own walls/cap. Rebuilt neighbours (an untagged floor
            # plane re-emitted over its OLD vertices) are never touched, and a
            # face that already carries a tag (or its own paint) keeps it. Runs
            # inside the commit's SnapshotMutation (and the reverted preview),
            # so undo is exact.
            #
            # Reached FROM the new vertices instead of by scanning the model:
            # a face is fresh exactly when it holds one, so the pass costs the
            # push rather than the mesh.
            fresh: set = set()
            made = push_scans.made_since(mesh, before_mark, "vertex")
            if made is None and before_verts is None:
                # Left the store meanwhile: the keys it had, from the mark.
                before_verts = set(before_mark.keys("vertex"))
            for v in (mesh.vertices if made is None else made):
                if made is not None or v.key not in before_verts:
                    fresh.update(v.faces())
            for f in fresh:
                if base_tag is not None and not f.attrs.get("ifc"):
                    f.attrs["ifc"] = dict(base_tag)
                if base_paint and not any(k in f.attrs for k in PAINT_KEYS):
                    f.attrs.update(base_paint)

    @staticmethod
    def _curve_runs(mesh, face):
        from ingetrazo_core import outline_curve_runs
        runs = []
        for loop in (face.loop, *face.hole_loops):
            ids = [getattr(mesh.find_edge(v, loop[(i + 1) % len(loop)]), "curve", None)
                   for i, v in enumerate(loop)]
            for indices, closed in outline_curve_runs(ids, True):
                runs.append(([QVector3D(loop[i].position) for i in indices], closed))
        return runs

    def _tag_moved_curves(self, mesh, runs) -> None:
        """The base's curves, carried to the moved cap: the ring a pushed
        circle leaves on top is a curve again, one click selects it all
        (#626). Only the cap's own edges are looked up -- the cost is the
        curve's, not the model's."""
        if not runs or self._normal is None:
            return
        shift = self._normal * self.extrusion
        for pts, closed in runs:
            moved = [mesh.vertex_at(p + shift) for p in pts]
            if any(v is None for v in moved):
                continue
            pairs = list(zip(moved, moved[1:]))
            if closed:
                pairs.append((moved[-1], moved[0]))
            edges = [mesh.find_edge(a, b) for a, b in pairs]
            if any(e is None or e.curve is not None for e in edges):
                continue
            cid = Mesh.next_curve_id()
            for e in edges:
                e.curve = cid

    # The dihedral above which a swept seam reads as a *curve* facet (a
    # cylinder's side) rather than a real corner. cos(31.8°): a 24-side circle
    # (15° steps) softens, a hexagon/octagon (≥45°) keeps its visible edges.
    _CURVE_FACET_COS = 0.85

    def _soften_curve_facets(self, mesh, before_edges, before_mark=None) -> None:
        """Hide the *new* edges between two faces that meet at a shallow dihedral
        — the vertical facet seams of a swept curve — so a pushed circle reads as
        a smooth cylinder while its circle outline (and a polygon's flat sides,
        which meet at a real angle) stay visible. Topology-free: only the render
        flag changes.

        ``before_mark``: a store snapshot (``push_scans.mark``) in place of
        ``before_edges`` on a store-backed mesh — the new edges are the keys
        it lacks, in list order (a view is in a set of views by its key)."""
        edges = mesh.edges
        if before_mark is not None:
            made = push_scans.made_since(mesh, before_mark, "edge")
            if made is None:
                # Materialised meanwhile: the views of before, as the set
                # of views would have held them.
                before_edges = set(push_scans.views(
                    mesh, "edge", before_mark.keys("edge")))
            else:
                edges, before_edges = made, ()
        for e in edges:
            if e in before_edges or e.soft or len(e.faces) != 2:
                continue
            d = QVector3D.dotProduct(e.faces[0].normal().normalized(),
                                     e.faces[1].normal().normalized())
            if self._CURVE_FACET_COS < d < 0.99995:
                e.soft = True

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
            translate_points(scene, {_key(p) for p in self._cap_positions},
                             tuple(n*d for n in normal.toTuple()) if "_store" in scene.mesh.__dict__
                             else normal * d)
            seed = list(self._cap_positions) + [p + normal * d
                                                for p in self._cap_positions]
            seedkeys = {_key(p) for p in seed}
            run_stitch(scene.mesh, seedkeys, set(), coplanar_merge=False)
            # A shrink can land the cap flush on a coplanar neighbour — a bump
            # pushed back level with its host wall, or a side flank pushed
            # clear across the bump onto the opposite flank. The weld may then
            # consume the pushed face itself (the coincident pair must vanish,
            # not survive as a zero-thickness fin), so the cleanup can't hinge
            # on it: run the same fixpoint plane rebuild as the extrude path
            # over every face the translation touched.
            if not _mesh_is_flat(scene.mesh):
                fresh = {f for f in scene.mesh.faces
                         if any(_key(v) in seedkeys for v in f.vertices)}
                self._rebuild_planes_fixpoint(scene.mesh, fresh, seedkeys,
                                              removing=d < 0)
            else:
                # A full collapse flattened the solid. The cap may have landed
                # on a *subdivided* base (hole-bearing cycles never dedupe as
                # identical): it is now a redundant mother over the
                # subdivision — exactly what the flat-drawing heal removes.
                heal_overlapping_faces(scene.mesh)
            # Deferred during drag PREVIEW frames, same as the extrude path
            # below: the state is discarded next frame and the shading is
            # winding-proof, so a parity pass per mouse move only added lag
            # (~10 s/frame on an imported 3k-face barbecue). The commit runs
            # it for real.
            if not preview:
                orient_outward(scene.mesh, only=self._touched_faces(
                    scene.mesh, seedkeys))
            return

        # Establish the outward invariant *up front* instead of trusting history:
        # hand-built or loaded meshes can arrive with mixed winding, and every
        # downstream decision (the wall quads' deterministic winding, the
        # per-plane rebuild's material-side classification) reads face normals.
        # If the base face flips, the drag distance flips with it so the push
        # still goes where the user dragged.
        #
        # PERF: on an OPEN mesh orient never flips a face — it only marks
        # interior partitions for the volumetric rebuild's parity queries — so
        # for open meshes the (expensive) pass is deferred until we know the
        # push takes the solid path at all; a pure sheet push skips it. The
        # per-drag flag skips repeating it every preview frame (the preview
        # snapshot preserves the oriented state). Re-orienting the eye model
        # each frame made the drag lag at ~4 fps.
        entry_closed = is_closed(scene.mesh)
        # ``d`` is signed along the normal the DRAG used, which is the base's
        # surface normal (see _surface_normal) — not necessarily the winding
        # the face carries. Reconcile the two, the same way this already
        # reconciles a face orient_outward flips underneath it: on an open
        # shell the two disagree, and taking the face's word for it swept the
        # push the wrong way, so a door previewed going IN and came out going
        # OUT at the commit (Marco, 2026-08-27).
        pre_normal = (self._normal if self._normal is not None
                      else face.normal())
        if entry_closed and not self._drag_pre_oriented:
            orient_outward(scene.mesh, only=(face,))
        normal = face.normal()
        if QVector3D.dotProduct(normal, pre_normal) < 0:
            d = -d
        # Only a vertex inside a base edge's box can split it (the boxes
        # refine_loop_with_points itself tests, grown): those, in list order.
        allverts = _refine_candidates(scene.mesh, [face.vertices, *face.holes])
        # Split the base loop at any existing vertex sitting on one of its edges
        # (a T-junction where the host wall ends and an earlier overhang's floor
        # begins) so each sub-edge is carried by one face and classifies right.
        base = refine_loop_with_points(face.vertices, allverts, mesh=scene.mesh)
        top = [v + normal * d for v in base]
        # A face with holes (an offset ring — the wall footprint) extrudes its
        # holes too: the inner walls rise and the cap keeps the opening, so a
        # ring lifts as walls-with-thickness, not the whole footprint.
        base_holes = [refine_loop_with_points(h, allverts, mesh=scene.mesh) for h in face.holes]
        top_holes = [[v + normal * d for v in h] for h in base_holes]
        count = len(base)
        faces = scene.faces
        memo: dict = {}
        attached_any, attached, sheet_fast, was_solid, through = push_region(
            base, base_holes, self._keep_base, entry_closed,
            lambda a, b: _classify_edge(face, a, b, scene.mesh, faces, memo)[0],
            lambda: self._find_through_face(face, d, faces, mesh=scene.mesh),
            lambda: _component_is_closed(scene.mesh, face), lambda: _mesh_is_flat(scene.mesh))
        if was_solid and not entry_closed and not self._drag_pre_oriented:
            # Open mixed mesh taking the solid path: mark interior partitions
            # for the rebuild's parity (orient never flips faces here).
            orient_outward(scene.mesh, only=(face,))

        # The faces there before, by key on a store-backed mesh (see
        # push_scans.set_minus: the same set, iteration order included).
        before_fkeys = push_scans.live_keys(scene.mesh, "face")
        before = set(scene.mesh.faces) if before_fkeys is None else None
        self._cap_cmd = None
        base_attrs = dict(face.attrs)
        if through is None and not attached_any and self._sweeps_a_free_sheet(
                face, base, base_holes, faces, mesh=scene.mesh):
            # A sheet swept free has no outside to orient against until it is
            # a block, and when it shares an edge with another sheet the block
            # is part of an open piece the orientation pass leaves alone: the
            # base stayed facing into the new block (#550). Wound outward by
            # construction instead — the base faces away from the sweep, the
            # cap and the sides along it.
            if d > 0:
                turn_face_over(face)
            else:
                base.reverse()
                top.reverse()
                for lp in (*base_holes, *top_holes):
                    lp.reverse()
        if through is not None:
            commands = self._through_commands(face, base, through,
                                              base_holes, faces)
        else:
            commands = self._extrude_commands(
                face, base, top, base_holes, top_holes, count, attached
            )
        from core.coordinate_frame import planned_points
        exact = {}
        if "_store" in scene.mesh.__dict__ and through is None:
            delta = tuple(n * d for n in normal.toTuple())
            for ring, raised in zip([base, *base_holes], [top, *top_holes]):
                for point, cap in zip(ring, raised):
                    vertex = scene.mesh.vertex_at(point)
                    if vertex is not None:
                        source = vertex.position_tuple()
                        exact[point.toTuple()] = source
                        exact[cap.toTuple()] = tuple(v + t for v, t in zip(source, delta))
        with planned_points(scene.mesh, exact):
            for cmd in commands:
                cmd.do(scene)
        if (base_attrs and self._cap_cmd is not None
                and self._cap_cmd.face in scene.mesh.faces):
            # The moved cap continues the (consumed) base: same attrs.
            self._cap_cmd.face.attrs = dict(base_attrs)
        far = getattr(self, "_far_cmd", None)
        self._far_cmd = None
        if far is not None and far[1] and far[0].face in scene.mesh.faces:
            far[0].face.attrs = dict(far[1])
        new_faces = None
        if before_fkeys is not None:
            now = push_scans.live_keys(scene.mesh, "face")
            if now is not None:
                new_faces = push_scans.set_minus(scene.mesh, "face", now,
                                                 set(before_fkeys))
            if new_faces is None:
                before = set(push_scans.views(scene.mesh, "face",
                                              before_fkeys))
        if new_faces is None:
            new_faces = set(scene.mesh.faces) - before
        if self._keep_base and attached_any and d < 0:
            # The Ctrl-stack grows *into* the solid: its cap is a deliberate
            # interior division (the inward counterpart of the kept base), not
            # a boundary declaration — mark it so the rebuild preserves it.
            for f in new_faces:
                fn = f.normal().normalized()
                if (abs(QVector3D.dotProduct(fn, normal)) > 0.999
                        and abs(QVector3D.dotProduct(
                            f.centroid() - top[0], normal)) < 1e-4):
                    f.interior = True
        seed = list(base) + list(top)
        for hb, ht in zip(base_holes, top_holes):
            seed += list(hb) + list(ht)
        seedkeys = {_key(p) for p in seed}
        # On a solid, the naive extrude's seams and overlaps are dissolved by the
        # deterministic per-plane rebuild below, so the winding-tolerant coplanar
        # merge (phase 3) stays off. On raw/open geometry (a flat sheet) there is
        # no "outside" to classify against, so the merge still applies there.
        solid = attached_any and was_solid
        # On solids the identical-cycle dedupe waits for the rebuild: a flush
        # collapse's sweep quad lands identical to the face it annihilates
        # with, and only the volumetric classification can tell "keep one"
        # (shared wall) from "drop both" (emptied region).
        run_stitch(scene.mesh, seedkeys, new_faces,
                   coplanar_merge=not solid and not self._keep_base,
                   dedupe=not solid)
        if solid:
            self._rebuild_planes_fixpoint(scene.mesh, set(new_faces), seedkeys,
                                          keep_mode=self._keep_base,
                                          removing=d < 0)
        # Give any closed solid a consistent outward orientation — every face's
        # normal pointing out. The extrude can otherwise commit a closed solid
        # wound inconsistently (a flipped cap, or the base of a first flat→solid
        # extrude), invisible until you push that face and it extrudes *inward*.
        # PERF: deferred during drag PREVIEW frames — the state is discarded
        # next frame and the face shading is winding-proof (abs), so paying a
        # parity pass per mouse move only made the drag lag; the commit runs
        # it for real. A sheet-path push whose result is still open has
        # nothing to orient either way (open meshes never flip; interior
        # marking is recomputed by the next solid op). What counts is the
        # pushed solid, not the whole mesh: an open piece elsewhere left a
        # new box with its base reversed (#230).
        if not preview:
            touched = self._touched_faces(scene.mesh, seedkeys)
            if (not sheet_fast or is_closed(scene.mesh)
                    or (touched and _component_is_closed(scene.mesh,
                                                         touched[0]))):
                orient_outward(scene.mesh, only=touched)

    def _cap_passes_a_wall_vertex(self, scene, face, d: float) -> bool:
        """Whether shrinking the prism cap by ``d`` carries it past a vertex
        partway up one of its walls — the next room's corner sitting on the
        wall. The translate leaves that vertex where it is, so the wall would
        fold into a spike around it (or a zero-area sliver at a full
        collapse); the extrude path cuts the wall instead."""
        if d >= 0:
            return False
        normal = self._normal if self._normal is not None else face.normal()
        from core.store_mesh import _store_of
        store = _store_of(scene.mesh)
        if store is not None:
            return store.cap_passes_wall(face.key, [p.toTuple() for p in self._cap_positions],
                                         normal.toTuple(), d)
        rim = {_key(p) for p in self._cap_positions}
        origin = self._cap_positions[0] if self._cap_positions else None
        if origin is None:
            return False
        walls = []
        for g in scene.mesh.faces:
            if g == face or not any(_key(v) in rim for v in g.vertices):
                continue
            walls.append([QVector3D.dotProduct(origin - v, normal)
                          for v in g.vertices if _key(v) not in rim])
        from ingetrazo_core import MeshStore
        return MeshStore.wall_depths_crossed(walls, d)

    @staticmethod
    def _sweeps_a_free_sheet(face, base, base_holes, faces, mesh=None) -> bool:
        """Whether ``face`` is a sheet whose whole rim — outline and holes —
        borders only other sheets on its plane or nothing at all: no face off
        its plane (a room's wall beside it) stands on any rim edge."""
        normal = face.normal()
        segs = [(lp[i], lp[(i + 1) % len(lp)]) for lp in (base, *base_holes)
                for i in range(len(lp))]
        near = ([push_scans.faces_on_segment(mesh, a, b) for a, b in segs]
                if mesh is not None else None)
        if near is not None and all(c is not None for c in near):
            # Only the faces near both ends of a rim edge can carry it.
            return not any(
                _segment_on_face_boundary(a, b, g)
                for (a, b), cands in zip(segs, near) for g in cands
                if g != face and not _faces_coplanar(normal, g.normal()))
        off_plane = [g for g in faces if g != face
                     and not _faces_coplanar(normal, g.normal())]
        for lp in (base, *base_holes):
            m = len(lp)
            for i in range(m):
                a, b = lp[i], lp[(i + 1) % m]
                if any(_segment_on_face_boundary(a, b, g) for g in off_plane):
                    return False
        return True

    @staticmethod
    def _touched_faces(mesh, seedkeys: set):
        """The faces carrying one of the operation's seed positions — the
        entry point into the solid(s) it edited, for scoping the orientation
        pass to those components. ``None`` (meaning "the whole mesh") when the
        op left none, so a collapse can never silently skip orientation."""
        from core.topology import KeyMemo
        key = KeyMemo()
        # Only a face with a corner near the seeds can hold one's key.
        near = push_scans.faces_near_points(mesh, seedkeys)
        hit = [f for f in (mesh.faces if near is None else near)
               if any(key(v) in seedkeys for v in f.vertices)]
        return hit or None

    @staticmethod
    def _rebuild_planes_fixpoint(mesh, fresh: set, seedkeys: set,
                                 keep_mode: bool = False,
                                 removing: bool = True) -> None:
        """Deterministic root-fix (path C): recompute each touched plane's
        faces from its edges — the planar arrangement finds every region,
        winding-classification keeps the ones inside the solid and drops
        phantoms outside, and the union dissolves coplanar seams. Seam planes
        (a fresh face coplanar-adjacent to another) cover the strip stacked on
        a wall and the quad overlapping a wall to be notched; crack planes
        cover anything a nested push left unfaced. Shared by the extrude path
        and the prism-translate path (whose flush landings need the same
        volumetric resolution).

        Rebuilt to a **fixpoint**: a plane's classification reads the current
        mesh, so a plane rebuilt while a neighbour still carries its dirty
        phantom can come out wrong — which made the result depend on iteration
        order (set order = the Python hash seed). Treating the faces a rebuild
        adds as fresh re-flags the affected planes next round, and
        ``apply_rebuild`` returning False on a stable plane makes the loop
        terminate. Plane keys are sorted, so the whole pass is
        order-deterministic. No case tree."""
        # The push's own rims, captured by *position* before any rebuild (the
        # objects die as rounds consolidate them): plane edges lying on none
        # of these are the user's hand-drawn subdivisions and survive the
        # union (A.4); edges on them are the op's seams and may dissolve.
        op_rims: list = []
        for f in fresh:
            for lp in (f.loop, *f.hole_loops):
                cnt = len(lp)
                for i in range(cnt):
                    op_rims.append((QVector3D(lp[i].position),
                                    QVector3D(lp[(i + 1) % cnt].position)))
        # Per-face memos (normal, centroid, triangles) and the packed parity
        # set, shared across every plane and round: faces are never mutated in
        # place inside this loop (rebuilds remove + add), so identity-keyed
        # entries stay valid. Without it each plane visit re-ran Newell over
        # the whole mesh — 76k normals per drag frame on the barbecue.
        cache = RebuildCache()
        from ingetrazo_core import rebuild_push_planes

        def planes(round_index):
            nonlocal fresh
            if round_index:
                fresh = {f for f in fresh if f in mesh.faces}
            cache.seed_faces = ElementSet(fresh)
            candidates = list(seam_planes(mesh, fresh))
            candidates.extend(crack_planes(mesh, _faces_window(fresh)))
            candidates.extend(_seam_planes_along(mesh, cache.dropped))
            cache.dropped = []
            return candidates

        def rebuild(pair):
            origin, normal = pair
            before = _FacesBefore(mesh)
            changed = apply_rebuild(mesh, origin, normal, fresh, keep_mode,
                                    removing, op=op_rims, cache=cache)
            if changed:
                fresh.update({f for f in before.new_faces(mesh) if not before.kept(f, fresh)})
            return changed

        rebuild_push_planes(planes, rebuild)
        # The dissolved seams can leave redundant collinear vertices on the
        # rebuilt faces' borders (the old cap ring's corners); one more
        # connectivity pass collapses them. No merge — rebuild already did.
        run_stitch(mesh, seedkeys, None, coplanar_merge=False)

    def _extrude_commands(self, face, base, top, base_holes, top_holes, count, attached):
        """Build the cap between the outer rim edges and the inner and outer walls."""
        commands = [DeleteFaceCommand(face)] if attached else []
        commands.append(_PushBand(base, top, rim=True, rails=True))
        self._cap_cmd = AddFaceCommand(top, auto=not attached, holes=top_holes)
        commands.append(self._cap_cmd)
        for lower, upper in zip(base_holes, top_holes):
            commands.append(_PushBand(lower, upper, rim=True, rails=True, walls=True))
        commands.append(_PushBand(base, top, walls=True))
        if attached:
            commands.append(PruneOrphanEdgesCommand(base))
        return commands

    # ---- Through-hole -------------------------------------------------------
    @staticmethod
    def _find_through_face(face, d, faces, mesh=None):
        return push_through_face(face, d, faces, mesh, push_scans.faces_meeting,
                                 loop_inside_face, subtract_loop_from_face)

    def _through_commands(self, face, base, through, base_holes=(), faces=()):
        """Punch the far face, connect the tunnel walls and close any swept islands."""
        far_face, back, remainder = through
        holes = list(far_face.holes)
        islands = []
        if remainder is None:
            holes.append(back)
            if base_holes:
                islands = self._through_islands(face, base_holes, far_face, back, faces)
        far = AddFaceCommand(far_face.vertices if remainder is None else remainder, auto=False, holes=holes)
        self._far_cmd = far, dict(far_face.attrs)
        commands = [DeleteFaceCommand(face), DeleteFaceCommand(far_face), far,
                    _PushBand(base, back, rim=remainder is None, rails=True, walls=True)]
        for lower, upper in islands:
            commands.append(_PushBand(lower, upper, rim=True, rails=True, walls=True, cap=True))
        commands.append(PruneOrphanEdgesCommand(list(base) + list(back)))
        return commands

    @staticmethod
    def _through_islands(face, base_holes, far_face, back_loop, faces):
        """``[(hole, back_hole)]`` for a ring pushed through: each hole of
        ``face`` with its copy on the far face's plane — or ``[]`` (no
        island geometry, the old behaviour) unless EVERY hole is filled by
        a single hole-free face on the same plane (the island), so an
        opening that already goes through (a window the ring frames) is
        never capped."""
        if not base_holes or len(back_loop) < 3:
            return []
        gn = far_face.normal().normalized()
        gc = far_face.centroid()
        # The push direction, from the opening's own sweep.
        lead = back_loop[0] - face.vertices[0]
        if lead.length() < 1e-9:
            return []
        push = lead.normalized()
        den = QVector3D.dotProduct(push, gn)
        if den < 1e-6:
            # Only through the face the push LEAVES the material by (it
            # faces along the push). An outward push that meets a face
            # looking back at it -- the eye model's iris raised past the
            # outer ring -- is not a ring carved through a slab.
            return []
        # The opening must land strictly INSIDE the far face, clear of its
        # rim and of its holes: an iris raised to an outer ring's cap meets
        # it along the cap's own hole, which is no slab to carve through.
        def _clear(p) -> bool:
            for lp in (far_face.vertices, *far_face.holes):
                k = len(lp)
                for i in range(k):
                    a, b = lp[i], lp[(i + 1) % k]
                    ab = b - a
                    L2 = ab.lengthSquared()
                    t = 0.0 if L2 < 1e-18 else max(0.0, min(
                        1.0, QVector3D.dotProduct(p - a, ab) / L2))
                    if (a + ab * t - p).length() < 1e-4:
                        return False
            return True
        if not all(_clear(p) for p in back_loop):
            return []
        out = []
        for hole in base_holes:
            m = len(hole)
            if m < 3:
                return []
            filled = None
            for i in range(m):
                kind, g = classify_push_edge(face, hole[i], hole[(i + 1) % m],
                                             faces)
                if kind != "coplanar" or g is None or g.holes \
                        or (filled is not None and g != filled):
                    return []
                filled = g
            if filled is None or len(filled.vertices) != m:
                return []
            back = []
            for p in hole:
                t = QVector3D.dotProduct(gc - p, gn) / den
                back.append(p + push * t)
            if not loop_inside_face(far_face, back):
                return []
            out.append((list(hole), back))
        return out

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
        self._preview_snapshot = None
        self._preview_selection = set()
        self._clear_stage()
        self._stage_cache = {}
        self._stage_refusal = None
        self._stage_slow = False
        self._plane_levels = []
        self._stage_serial = None
        self._drag_pre_oriented = False
        self._inference_point = None
        self._inference_kind = None
        self._infer_cache = None       # per-drag projected-vertex candidates
        self._infer_boxes = None       # ... and where they fall on screen
        self._refused = False
        self._tiny_hint_shown = False
        self._press_screen = None
        self._last_screen = None
        self._light_faces = []
        self._light_rings = []
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
