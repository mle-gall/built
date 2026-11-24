# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Topology helpers — graph queries over the scene's edge network.

Used by tools (today: ``LineTool``) to find polygons that close when a new
edge is added. Modeled after the classic behaviour: as soon as a new edge
completes a planar cycle in the edge graph — using any combination of
existing edges — that cycle becomes a face automatically.

Position equality is tolerant: two vertices within ``_KEY_DECIMALS``
decimal places (≈ 0.1 mm at metric scale) are treated as the same node.
"""
from __future__ import annotations

import ingetrazo_core as kernel

from typing import Iterable, Optional

from ingetrazo_core import Vec3 as QVector3D, polygon_is_planar as is_planar

from core.geometry import Edge, Face


_KEY_DECIMALS = 4
_PLANAR_TOLERANCE = 1e-3
# Two points closer than this weld together; also the gate for deciding a
# crossing is real (skew lines whose closest approach exceeds it don't touch).
_SPLIT_TOLERANCE = 1e-4


def _key(p: QVector3D) -> tuple[float, float, float]:
    if not hasattr(p, "toTuple"):
        return tuple(round(x, _KEY_DECIMALS) for x in p)
    return (round(p.x(), _KEY_DECIMALS),
            round(p.y(), _KEY_DECIMALS),
            round(p.z(), _KEY_DECIMALS))


class KeyMemo:
    """``_key`` memoised per point OBJECT for the length of one scan. A push
    on a 25 797-face group called ``_key`` 655 000 times (2 M ``round``) —
    each vertex once per face and edge it belongs to. Only for a scan that
    moves nothing: a position mutated in place keeps its object."""

    __slots__ = ("_seen",)

    def __init__(self) -> None:
        self._seen: dict = {}

    def __call__(self, p: QVector3D) -> tuple:
        hit = self._seen.get(id(p))
        if hit is not None and hit[0] is p:
            return hit[1]
        k = _key(p)
        self._seen[id(p)] = (p, k)
        return k


def same_position(p: QVector3D, q: QVector3D) -> bool:
    """Whether two points coincide within the welding tolerance (≈ 0.1 mm)."""
    return _key(p) == _key(q)


find_duplicate_edge = kernel.object_drawing.find_duplicate_edge


def find_smallest_cycle_through(
    edges: Iterable[Edge],
    a: QVector3D,
    b: QVector3D,
    max_len: int = 32,
) -> Optional[list[QVector3D]]:
    """Smallest simple cycle in the edge graph that contains segment ``a-b``.

    The segment is *virtual*: it does not need to exist in ``edges`` yet.
    Returns the cycle as an ordered list of vertices starting at ``a`` and
    walking back to ``b`` through existing edges (so the full polygon loop
    is the returned list with the implicit closing segment back to ``a``).
    Returns ``None`` if no cycle exists or it would exceed ``max_len`` nodes.
    """
    ka, kb = _key(a), _key(b)
    if ka == kb:
        return None
    cycles = _drawing_cycles(edges, a, b, max_len, False)
    if cycles is not NotImplemented:
        return cycles[0] if cycles else None
    return _smallest_cycle(_cycle_adjacency(edges, ka, kb), a, b, ka, kb,
                           max_len)


def _store_edge_plan(edges):
    from core.store_mesh import _Seq, StoreMesh
    if isinstance(edges, _Seq) and edges._kind == "edge" and type(edges._m) is StoreMesh:
        from core import scan_index
        from core.edits import _ScanEdges
        sc = scan_index.scan(edges._m)
        if sc is not None:
            return _ScanEdges(sc)
    return None


def _drawing_cycles(edges, a, b, limit, both):
    edges = _store_edge_plan(edges) or edges
    query = getattr(edges, "drawing_cycles", None)
    return NotImplemented if query is None else query(a, b, limit, both)


def _cycle_adjacency(edges, ka, kb, key=_key) -> dict:
    """``adj[u] -> [(v_key, v_pos), …]`` of the edge graph, in edge order,
    without an existing copy of segment ``ka``–``kb`` (the cycle would just
    find itself, a length-2 loop a→b→a)."""
    return kernel.object_drawing.graph_adjacency(edges, ka, kb, key)


class CycleGraph:
    """The edge graph ``find_cycles_through`` searches, kept for a whole
    plan instead of built again for each segment of the stroke: a circle
    of 32 sides drawn on a solid of 6000 edges walked them 32 times.

    Unlike :func:`_cycle_adjacency` it holds every edge, the segment asked
    about included; the search leaves that one out as it goes (``direct``
    in :func:`_smallest_cycle`). ``add`` and ``remove`` follow the plan's
    edge list, so each node's neighbours stay in that list's order — the
    order the search's answer depends on."""

    __slots__ = ("_adj", "_key", "_scan")

    def __init__(self, edges, key=_key) -> None:
        self._adj: dict = {}
        self._key = key
        self._scan = _store_edge_plan(edges) if key is _key or isinstance(key, KeyMemo) else None
        if self._scan is not None:
            return
        for edge in edges:
            self.add(edge)

    @property
    def adj(self):
        if self._scan is not None:
            sc = self._scan
            self._scan = None
            for edge in sc.all_elements():
                self.add(edge)
        return self._adj

    def add(self, edge) -> None:
        if self._scan is not None:
            self._scan.append(edge)
            return
        kernel.object_drawing.graph_insert(self.adj, self._key, edge)

    def remove(self, edge) -> bool:
        """Take ``edge`` out. False if it was not found as it was added —
        the graph is then not to be trusted and the caller drops it."""
        if self._scan is not None:
            for candidate in self._scan.pick((edge.a, edge.b)):
                if candidate == edge:
                    self._scan.remove(candidate)
                    return True
            return False
        return kernel.object_drawing.graph_remove(self.adj, self._key, edge)


_smallest_cycle = kernel.object_drawing.smallest_cycle


def segment_intersection(
    p1: QVector3D,
    p2: QVector3D,
    p3: QVector3D,
    p4: QVector3D,
    tol: float = _SPLIT_TOLERANCE,
) -> Optional[QVector3D]:
    """Where segment ``p1-p2`` meets segment ``p3-p4`` in 3D, or ``None``.

    Uses the closest-points-between-two-lines solution and accepts the hit
    only when (a) the lines are not parallel, (b) their closest approach is
    within ``tol`` (so genuinely skew segments that merely *look* crossed in
    a 2D projection are rejected), and (c) both parameters land on their
    segment (endpoints included). The returned point is the midpoint of the
    closest approach, so an X-crossing yields one shared vertex for both
    edges. Collinear overlaps return ``None`` — those are a merge problem,
    handled separately, not a crossing.
    """
    return kernel.segment_hit(p1, p2, p3, p4, tol)


def plan_edge_split(
    edges: Iterable[Edge], a: QVector3D, b: QVector3D
) -> tuple[list[tuple[QVector3D, QVector3D]], dict[Edge, QVector3D]]:
    """Plan the splits caused by adding segment ``a-b`` to ``edges``.

    Returns a pair:

    - ``new_segments`` — the new edge broken at every interior crossing,
      ordered from ``a`` to ``b`` (just ``[(a, b)]`` when nothing is crossed);
    - ``edge_cuts`` — existing edge → the interior point where the new edge
      crosses it (those edges must be replaced by two sub-edges).

    A crossing at a shared *endpoint* produces no split on that side (the
    weld already shares that vertex). A straight segment meets another at
    most once, so each existing edge maps to a single cut point.
    """
    plan = _store_edge_plan(edges)
    if plan is not None:
        return plan.split_plan(a, b, whole=True)
    return kernel.split_edges(edges, a, b)


def face_exists(faces: Iterable[Face], cycle: list[QVector3D]) -> bool:
    """Whether a face with the same vertex set as ``cycle`` already exists."""
    cycle_keys = frozenset(_key(v) for v in cycle)
    for face in faces:
        if frozenset(_key(v) for v in face.vertices) == cycle_keys:
            return True
    return False


# ---- Containment (face split / hole punching) ------------------------------

def _on_segment_2d(p, a, b, tol: float = 1e-7) -> bool:
    """Whether 2D point ``p`` lies on segment ``a``–``b`` within ``tol``."""
    cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    if abs(cross) > tol:
        return False
    dot = (p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1])
    if dot < -tol:
        return False
    sqlen = (b[0] - a[0]) ** 2 + (b[1] - a[1]) ** 2
    return dot <= sqlen + tol


def _strictly_inside_2d(p, poly: list[tuple[float, float]]) -> bool:
    """Ray-cast point-in-polygon, strict: points on the boundary are *not*
    inside (they signal a shared-edge case, which is a chord split, not a
    hole)."""
    return kernel.polygon_contains_2d(p, poly, True)


def _drawing_store(face):
    from core.store_mesh import FaceView, StoreMesh
    if isinstance(face, FaceView) and type(face._m) is StoreMesh:
        from core.store_mesh import _hypot
        if _hypot() is None:
            return None
        return face._m._store
    return None


def loop_inside_face(mother: Face, loop: list[QVector3D],
                     outside_holes: bool = False) -> bool:
    """Whether ``loop`` lies entirely, strictly inside coplanar face ``mother``.

    Requires the loop to be coplanar with the mother and every loop vertex to
    fall strictly inside the mother's outer polygon (no vertex on its
    boundary). A loop that shares any boundary point is a chord split, handled
    elsewhere, and returns ``False`` here.

    With ``outside_holes`` the mother's holes count as *not hers*: a loop with
    a vertex inside one of her holes is not contained (it belongs to whatever
    face fills that hole — the nested-rectangle case).
    """
    store = _drawing_store(mother)
    if store is not None:
        from core._native import qt_hypot
        return store.loop_inside_face(mother.key, [p.toTuple() for p in loop], outside_holes, qt_hypot())
    return kernel.FaceOutline(mother).contains(loop, outside_holes)


_loops_cross_2d = kernel.loops_cross


def hole_merged_with_loop(face: Face, loop: list[QVector3D]):
    """Return the hole index and grown rim when a drawn loop shares a boundary."""
    store = _drawing_store(face)
    if store is not None:
        from core._native import qt_hypot
        return store.merged_hole(face.key, [p.toTuple() for p in loop], qt_hypot())
    return kernel.FaceOutline(face).merged_hole(loop)


def find_containing_face(
    faces: Iterable[Face], loop: list[QVector3D], exclude: Optional[Face] = None
) -> Optional[Face]:
    """Smallest existing face that strictly contains ``loop`` (or ``None``).

    Hole-aware: a loop inside a candidate's *hole* is not contained by it (it
    belongs to the face filling that hole — drawing a rectangle inside an
    already-drawn rectangle must punch the inner face, not the grandmother
    whose hole already excludes the spot). Smallest by vertex count is a
    cheap, good-enough proxy for the immediate mother when faces are nested.
    ``exclude`` skips the face being added.
    """
    from core.store_mesh import _Seq, FaceView, StoreMesh, _hypot
    if isinstance(faces, _Seq) and type(faces._m) is StoreMesh:
        mesh, candidates = faces._m, None
    elif isinstance(faces, (list, tuple)) and faces and _drawing_store(faces[0]) is not None:
        mesh = faces[0]._m
        candidates = [f.key for f in faces if isinstance(f, FaceView) and f._m is mesh]
        if len(candidates) != len(faces):
            mesh = None
    else:
        mesh = None
    if mesh is not None and (hypot := _hypot()) is not None:
        k = mesh._store.containing_face([p.toTuple() for p in loop], candidates,
                                        getattr(exclude, "key", None), hypot)
        return None if k is None else FaceView(mesh, k)
    return kernel.containing_object(faces, loop, exclude)


_loop_edges = kernel.loop_edges


def orphaned_edges_at(
    edges: Iterable[Edge], faces: Iterable[Face], vertices: Iterable[QVector3D]
) -> list[Edge]:
    """Edges incident to any of ``vertices`` that border no face.

    Used after push/pull consumes a face to sweep up the dangling lines left
    where geometry was carved away, without disturbing standalone edges
    elsewhere (only those touching the operation's vertices are considered).
    """
    from core.store_mesh import _Seq, StoreMesh, EdgeView
    if (isinstance(edges, _Seq) and isinstance(faces, _Seq)
            and edges._m is faces._m and type(edges._m) is StoreMesh):
        mesh = edges._m
        keys = mesh._store.orphaned_edges_at([p.toTuple() for p in vertices], None, None)
        return [EdgeView(mesh, k) for k in keys]
    return kernel.orphan_objects(edges, faces, vertices)


def _faces_coplanar(n1: QVector3D, n2: QVector3D) -> bool:
    return abs(QVector3D.dotProduct(n1.normalized(), n2.normalized())) > 0.999


_face_all_edges = kernel.face_edges
_segment_on_face_boundary = kernel.boundary_carries
_point_on_seg_incl = kernel.object_drawing.point_on_segment


def refine_loop_with_points(
    loop: list[QVector3D], points: Iterable[QVector3D], tol: float = _SPLIT_TOLERANCE, *, mesh=None
) -> list[QVector3D]:
    """Insert any ``points`` that lie on an edge's interior into ``loop``.

    A pushed wall whose edge runs along a *T-junction* — where the host wall ends
    and an earlier overhang's floor begins — has that edge covered piecewise by
    two faces, but no single one, so it reads as ``free``. Splitting the edge at
    the existing vertices sitting on it makes each sub-segment carried by one
    face, so push/pull classifies (and consumes) it correctly. Returns a new loop
    with the collinear points inserted in order; the originals are untouched."""
    from core.store_mesh import StoreMesh, _hypot
    if type(mesh) is StoreMesh and (hypot := _hypot()) is not None:
        ring = mesh._store.refine_ring([p.toTuple() for p in loop],
                                       [p.toTuple() for p in points], tol, hypot)
        return [QVector3D(*p) for p in ring]
    return kernel.refined_ring(loop, points, tol)


def classify_push_edge(
    face: Face, a: QVector3D, b: QVector3D, faces: Iterable[Face],
    memo: Optional[dict] = None,
) -> tuple[str, Optional[Face]]:
    """How a push/pull side edge ``a``–``b`` of ``face`` attaches to the model.

    - ``("coplanar", g)`` — a face on the same plane shares this edge (its
      boundary or a hole); the push raises an inner wall here.
    - ``("perp", s)`` — a non-coplanar face carries this edge on its boundary
      (the solid's side wall); pushing in notches that wall.
    - ``("free", None)`` — nothing adjacent; a free extrusion edge.

    A coplanar neighbour is matched even when it carries the edge only as a
    *sub-segment* of a wider boundary edge (its own edge was never split at this
    vertex — the colinear-overlap case). Without this, a block flush against a
    wider wall reads its shared edge as ``free`` and the push becomes a stray
    free extrusion that leaves the base as an internal partition.

    ``memo`` (a dict the caller keeps while it asks about the edges of ONE
    face against an unchanged model) remembers which faces are coplanar
    with ``face`` and the edge set of each: a cap holed by sixty cylinders
    has 2000 edges, and listing them again for each of the 32 edges of a
    circle pushed on it was a third of the push.
    """
    store = _drawing_store(face)
    if store is not None:
        from core.store_mesh import _Seq, FaceView, _hypot
        if isinstance(faces, _Seq) and faces._m is face._m:
            candidates = None
        else:
            faces = list(faces)
            if not all(isinstance(f, FaceView) and f._m is face._m for f in faces):
                store = None
            candidates = [f.key for f in faces] if store is not None else []
        if store is not None:
            kind, k = store.classify_segment(face.key, a.toTuple(), b.toTuple(), candidates, _hypot())
            return kind, None if k is None else FaceView(face._m, k)
    return kernel.classify_object_edge(face, a, b, faces, memo)


def face_is_bordered(face: Face, faces: Iterable[Face]) -> bool:
    """Whether every boundary edge of ``face`` is also an edge of some other
    face (its boundary or a hole).

    A bordered face is embedded in a surface or solid — a cube's top, or a
    rectangle drawn inside another face — so push/pull *moves* it and the base
    is consumed (carving a recess, or extending/shortening a solid without
    leaving an internal cap). A free-standing face (free edges) is *extruded*,
    keeping the base as a cap. This is orientation-independent, so it works
    regardless of how the face happens to be wound.
    """
    return kernel.bordered_object(face, faces)


# ---- Chord split (a new edge divides an existing face) ---------------------

def _locate_on_loop(vertices, point):
    return kernel.locate_boundary(vertices, point)


def split_face_by_chord(
    face: Face, a: QVector3D, b: QVector3D
) -> Optional[tuple[list[QVector3D], list[QVector3D]]]:
    """If segment ``a``–``b`` is a chord of ``face`` (both ends on its
    boundary, the segment running through its interior), return the two
    sub-loops it divides the face into; otherwise ``None``.

    Handles ends that are existing vertices *or* points on a boundary edge
    (the latter get inserted into the loop). Faces with holes are skipped —
    chord-splitting a holed face is a harder case left for later. The two
    returned loops inherit the mother's winding, so neither comes out
    inverted.
    """
    store = _drawing_store(face)
    if store is not None:
        from core._native import qt_hypot
        pieces = store.split_face_loops(face.key, [a.toTuple(), b.toTuple()], True, qt_hypot())
        return None if pieces is None else tuple([QVector3D(*p) for p in ring] for ring in pieces)
    return kernel.FaceOutline(face).split((a, b), True)


def find_chord_split(
    faces: Iterable[Face], a: QVector3D, b: QVector3D
) -> Optional[tuple[Face, list[QVector3D], list[QVector3D]]]:
    """First face that segment ``a``–``b`` chord-splits, with its two halves."""
    for face in faces:
        result = split_face_by_chord(face, a, b)
        if result is not None:
            return face, result[0], result[1]
    return None


def _loop_with_point(
    loop: list[QVector3D], ka: tuple, kb: tuple, point: QVector3D
) -> Optional[list[QVector3D]]:
    """Copy of ``loop`` with ``point`` inserted between the first consecutive
    vertex pair whose keys are ``ka``/``kb`` (either order). ``None`` if no such
    boundary edge exists or ``point`` already is one of those vertices."""
    kp = _key(point)
    n = len(loop)
    for i in range(n):
        j = (i + 1) % n
        ki, kj = _key(loop[i]), _key(loop[j])
        if {ki, kj} == {ka, kb}:
            if kp in (ki, kj):
                return None
            new = list(loop)
            new.insert(i + 1, QVector3D(point))
            return new
    return None


def split_edge_in_faces(
    faces: Iterable[Face],
    edge_a: QVector3D,
    edge_b: QVector3D,
    point: QVector3D,
    skip_endpoints: Iterable[QVector3D] = (),
) -> list[tuple[Face, list[QVector3D]]]:
    """Propagate an edge split into the faces that share that edge.

    When a drawn segment cuts an existing edge ``edge_a``–``edge_b`` at
    ``point``, every face carrying that edge on its outer boundary should gain
    ``point`` as a (collinear) vertex — otherwise the face stays detached from
    the split and won't follow when that vertex is later moved. This is what
    lets a gable wall pick up the ridge apex and deform into a pentagon when the
    ridge is raised, instead of leaving an open triangular gap.

    Returns ``[(face, new_vertices), ...]`` for the faces that changed. A face
    carrying *all* of ``skip_endpoints`` on its boundary is left out: that face
    is chord-split by the drawn segment, which already inserts the point, so
    handling it here would double-process it. Holes are left untouched.
    """
    ka, kb = _key(edge_a), _key(edge_b)
    skip = list(skip_endpoints)
    out: list[tuple[Face, list[QVector3D]]] = []
    for f in faces:
        if skip and all(_locate_on_loop(f.vertices, p) is not None for p in skip):
            continue
        new_verts = _loop_with_point(f.vertices, ka, kb, point)
        if new_verts is not None:
            out.append((f, new_verts))
    return out


# ---- Loop subtraction (a face drawn against another's boundary) -------------

def find_subdividing_chain(
    face: Face, loop: list[QVector3D]
) -> Optional[list[QVector3D]]:
    """If ``loop`` shares a contiguous boundary arc with ``face`` and pushes a
    single run of vertices through its interior, return that interior chain
    ``[P, ...interior..., Q]`` (P, Q on the face boundary). Otherwise ``None``.

    This is the "rectangle drawn in a corner / along an edge" case: the loop
    neither sits strictly inside the face (a hole) nor is a single straight
    chord — it carves a connected sub-region. Loops that poke outside the face
    or leave its plane, and loops touching the boundary in more than one place
    are out of scope and return ``None``.

    Holes on ``face`` are allowed and ignored here (the chain only concerns the
    outer boundary): drawing a door on a wall that already has a window must
    still subdivide. The caller is responsible for re-assigning each hole to the
    region that contains it.
    """
    store = _drawing_store(face)
    if store is not None:
        from core._native import qt_hypot
        chain = store.subdividing_chain(face.key, [p.toTuple() for p in loop], qt_hypot())
        return None if chain is None else [QVector3D(*p) for p in chain]
    return kernel.FaceOutline(face).chain(loop)


def split_face_by_chain(
    face: Face, chain: list[QVector3D]
) -> Optional[tuple[list[QVector3D], list[QVector3D]]]:
    """Split ``face`` along ``chain`` (``[P, ...interior..., Q]``, ends on the
    boundary) into its two sub-loops. On-edge ends are inserted into the
    boundary. Returns ``None`` if the ends can't be located."""
    store = _drawing_store(face)
    if store is not None:
        from core._native import qt_hypot
        pieces = store.split_face_loops(face.key, [p.toTuple() for p in chain], False, qt_hypot())
        return None if pieces is None else tuple([QVector3D(*p) for p in ring] for ring in pieces)
    return kernel.FaceOutline(face).split(chain, False)


def subtract_loop_from_face(
    face: Face, loop: list[QVector3D]
) -> Optional[list[QVector3D]]:
    """The remainder of ``face`` after ``loop`` is carved out of it, when the
    loop shares a contiguous boundary arc (corner / edge rectangle). ``None``
    if it isn't that case. The ``loop`` itself stays a separate face."""
    chain = find_subdividing_chain(face, loop)
    if chain is None:
        return None
    split = split_face_by_chain(face, chain)
    if split is None:
        return None
    r1, r2 = split
    loop_keys = frozenset(_key(v) for v in loop)
    if frozenset(_key(v) for v in r1) == loop_keys:
        return r2
    if frozenset(_key(v) for v in r2) == loop_keys:
        return r1
    return None  # neither half is the drawn loop → ambiguous, leave it alone


def carve_loop_by_chords(
    face: Face, loop: list[QVector3D]
) -> Optional[list[list[QVector3D]]]:
    """The pieces of ``face`` left around ``loop`` when EVERY corner of the
    loop sits on the face's boundary — a band drawn across a face from one
    side to the other (a rectangle across a stair tread, from its nosing to
    the riser). ``find_subdividing_chain`` needs a corner inside the face,
    so the Rectangle tool left the face whole with the band lying on top:
    two overlapping faces, edges with one and three faces, and a Push/Pull
    that built a broken solid (found testing #94). Lines drawn one by one
    never hit it: each chord splits the face as it lands.

    The loop's sides that cross the face (chords: midpoint strictly inside)
    split it one by one; the piece that IS the loop is dropped (it stays a
    face of its own). Returns the remaining outer loops, or ``None`` when
    this is not that case."""
    store = _drawing_store(face)
    if store is not None:
        from core._native import qt_hypot
        pieces = store.carve_face(face.key, [p.toTuple() for p in loop], qt_hypot())
        return None if pieces is None else [[QVector3D(*p) for p in ring] for ring in pieces]
    return kernel.FaceOutline(face).carve(loop)


# ---- Multiple-cycle detection ----------------------------------------------

def _same_cycle(c1: list[QVector3D], c2: list[QVector3D]) -> bool:
    return frozenset(_key(v) for v in c1) == frozenset(_key(v) for v in c2)


def find_cycles_through(
    edges: Iterable[Edge], a: QVector3D, b: QVector3D, max_results: int = 2,
    key=None, graph: Optional["CycleGraph"] = None,
) -> list[list[QVector3D]]:
    """Up to ``max_results`` distinct minimal cycles through segment ``a``–``b``.

    A single new edge can close more than one face — the classic case being a
    diagonal across a square, which bounds a triangle on each side. The first
    cycle is the smallest; the second is the smallest found after removing the
    first's interior nodes, which routes the search to the other side. This is
    what stops auto-facing from creating only one of the two triangles.
    """
    # One adjacency for both searches: the second one (the other side)
    # skips the first cycle's interior nodes instead of rebuilding the graph
    # without them — the same search, since the lists keep the edge order.
    # ``key`` (a KeyMemo) keys each vertex once per plan; a 38 700-edge group
    # rounded every endpoint twice per stroke (09-10).
    key = key or _key
    ka, kb = _key(a), _key(b)
    if ka == kb:
        return []
    # ``graph`` is ``edges`` already laid out, by a caller that asks about
    # one segment after another over the same list.
    if graph is not None and graph._scan is not None:
        return graph._scan.drawing_cycles(a, b, 32, max_results >= 2)
    if graph is None:
        cycles = _drawing_cycles(edges, a, b, 32, max_results >= 2)
        if cycles is not NotImplemented:
            return cycles
    direct = graph is not None
    if direct:
        adj = graph.adj
    else:
        # The planner's simulated edge list over a mesh scan builds each
        # node's list when the search reaches it (``core.edits._ScanEdges``):
        # the same lists, without walking the half-million edges of a big
        # mesh per click.
        lazy = getattr(edges, "cycle_adjacency", None)
        adj = (lazy(ka, kb) if lazy is not None
               else _cycle_adjacency(edges, ka, kb, key))
    first = _smallest_cycle(adj, a, b, ka, kb, direct=direct)
    if first is None:
        return []
    cycles = [first]
    if max_results >= 2:
        interior = {_key(v) for v in first} - {ka, kb}
        if interior:
            second = _smallest_cycle(adj, a, b, ka, kb, skip=interior,
                                     direct=direct)
            if second is not None and not _same_cycle(second, first):
                cycles.append(second)
    return cycles


# ---- Polygon offset (Offset tool: walls with thickness) --------------------

def offset_loop(loop, normal, d):
    """Slide and intersect a closed polygon, rejecting collapsed or reversed edges."""
    from ingetrazo_core import ToolMachine
    points = ToolMachine.offset_loop([p.toTuple() for p in loop], normal.toTuple(), d)
    return None if points is None else [QVector3D(*p) for p in points]


def max_offset_distance(loop: list[QVector3D], normal: QVector3D,
                       sign: float = 1.0, limit: float = 10.0) -> float:
    """The largest offset in the direction of ``sign`` that :func:`offset_loop`
    still accepts, in metres (0.0 when even a hair collapses the loop).

    Exists so the tool can say WHY it refused. A 20 cm kerb cannot take a
    10 cm inward offset — 0.20 - 2 x 0.10 is exactly zero — and a paved slab
    whose narrowest side is 4.6 cm inverts long before 10 cm. Refusing is
    right; refusing in silence is what reads as "the tool doesn't work"
    (Marco, 2026-09-10). Bisection: offset_loop is cheap and this only runs
    on the failure path.
    """
    step = abs(limit)
    if offset_loop(loop, normal, sign * 1e-4) is None:
        return 0.0
    lo, hi = 1e-4, step
    for _ in range(40):
        mid = (lo + hi) / 2.0
        if offset_loop(loop, normal, sign * mid) is None:
            hi = mid
        else:
            lo = mid
    return lo


# ---- Heal overlapping coplanar faces (spurious mother) ---------------------

def _loop_inside_loop(inner: list[QVector3D], outer: list[QVector3D],
                      normal: QVector3D) -> bool:
    """Whether ``inner`` lies inside ``outer`` (both coplanar loops, by centroid)."""
    import sys
    return kernel.inside_loop(inner, outer, normal, sys.version_info >= (3, 12))


_interior_point = kernel.interior_point
_HolesApart = kernel.HoleBounds


def _point_inside_2d(pt, poly) -> bool:
    return kernel.polygon_contains_2d(pt, poly, False)


def _maximal_holes(holes: list) -> list:
    """Drop holes nested inside a larger hole of the same face — redundant
    overlapping holes that incremental subdivision can leave behind."""
    mask = kernel.object_hole_mask(holes)
    return [hole for hole, keep in zip(holes, mask) if keep]


def _point_in_face_solid(face: Face, point: QVector3D) -> bool:
    """Whether ``point`` lies in ``face``'s solid region — inside its outer loop
    and outside every hole."""
    return kernel.FaceOutline(face).solid_at(point)


def winding_footprint(mesh):
    """How the faces of a flat drawing face BEFORE an edit, for
    :func:`orient_coplanar_faces` to respect afterwards: ``(face, normal,
    outer, holes)`` per face, positions copied (a face the edit removes may
    not keep its loop), plus its bounding box for a cheap first test.
    ``None`` where the orientation pass never runs — a 3D model, or one
    past the heal cap."""
    if len(mesh.faces) > _HEAL_FACE_CAP or not _mesh_is_flat(mesh):
        return None
    from core.store_mesh import StoreMesh, _hypot
    if type(mesh) is StoreMesh and _hypot() is not None:
        return mesh._store.snapshot()
    return kernel.winding_footprint(mesh.faces)


def orient_coplanar_faces(mesh, footprint=None) -> list:
    """Flip faces whose winding came out reversed, so coplanar faces face the
    same way.

    Auto-faced cycles can close with the opposite orientation (a normal pointing
    the wrong way), and push/pull then extrudes that face *into* the model
    instead of out — it looks like "I can't push this one". A valid solid never
    has two anti-parallel faces on the same plane, so flipping any face whose
    normal opposes the area-weighted majority of its plane only ever fixes that
    anomaly. Returns the flipped faces.

    With ``footprint`` (:func:`winding_footprint`, taken before the edit) the
    orientation is the USER's: a face that was already there is never turned,
    and a face the edit made — a split half, a rebuilt region — faces the way
    the face it was cut from did. Only a region drawn where no face was
    follows the majority. Without it, a face reversed on purpose (Reverse
    Faces) came back the other way at the next draw on the plane — a loose
    rectangle anywhere was enough.
    """
    from core.store_mesh import StoreMesh, FaceView, _hypot
    import sys
    if type(mesh) is StoreMesh and (footprint is None or hasattr(footprint, "namespace")) and (hypot := _hypot()) is not None:
        keys = mesh._store.winding_flips(hypot, sys.version_info >= (3, 12), footprint)
        flipped = [FaceView(mesh, k) for k in keys]
    else:
        if hasattr(footprint, "namespace"):
            # A legacy command materialised the mesh after taking the footprint.
            from core.store_mesh import become_store
            from core.mesh import Mesh
            from core import _native
            old = Mesh()
            store = _native.native.MeshStore(footprint.namespace)
            store.restore(footprint, footprint.namespace)
            become_store(old, store, [])
            footprint = kernel.winding_footprint(old.faces)
        flipped = kernel.object_winding(mesh.faces, footprint)
    for face in flipped:
        outer = list(reversed(face.vertices))
        holes = face.holes
        attrs = dict(face.attrs)
        mesh.remove_face(face)
        mesh.add_face(outer, holes or None).attrs = attrs
    return flipped


def sweep_tjunctions(mesh, seedkeys=None) -> None:
    """Split every edge that runs past a vertex belonging to another face, in
    ONE vectorised pass per round instead of a Python walk per edge.

    ``seedkeys`` (``core.topology._key`` tuples — positions rounded to the weld
    tolerance, NOT ``core.mesh._key``'s integer cells) bound the sweep to an
    operation's own box: only an edge reaching that box can be split, which is
    every T-junction the op can have created and none it merely found lying
    around. ``None`` sweeps the whole mesh, which is what a repair pass wants.

    The scalar form was O(splits x E x V) — ``interior_vertex_on`` walked EVERY
    vertex in Python and the loop restarted from edge zero after each split. On
    the imported brick barbecue (3054 faces) one pass alone measured 24.8 s
    (6983 edges x 3993 vertices), and the pass repeats per split: that is the
    hang Marco hit deleting a few faces inside it, and the same hang the push
    preview hit before phase 1 was batched. ONE implementation now serves both.
    """
    from core import scan_index
    from core.mesh import _STITCH_TOL
    seeds = list(seedkeys or ())
    while True:
        scan = scan_index.scan(mesh) or scan_index.Scan(mesh)
        hits = scan.native.stitch_hits(seeds, _STITCH_TOL)
        if not hits:
            return
        edges, vertices = zip(*hits)
        for edge, vertex in zip(scan.edges(edges), scan.vertices(vertices)):
            mesh.split_edge_at(edge, vertex)


def resolve_tjunctions(mesh, max_iter: int = 1000) -> None:
    """Split edges at T-junction vertices so faces with mismatched subdivisions
    share connectivity instead of a naked collinear seam.

    Two walls meeting can leave one face's long edge running past the vertices
    where the other face is split (the door, a perpendicular wall). Their shared
    boundary is then two separate naked edges, not one border-2 edge — so erasing
    the dividing line cascades and deletes a wall instead of merging. Splitting at
    each interior vertex welds the seam; erase-merge then reunites the walls.

    Whole-mesh repair, so it sweeps unscoped — but through the same batched
    pass the push stitch uses, not the per-edge Python walk that hung on a
    3k-face import. ``max_iter`` is kept for callers; the sweep runs to its
    own fixpoint."""
    sweep_tjunctions(mesh)


def fold_nonplanar_faces(mesh, tolerance: float = _PLANAR_TOLERANCE,
                         moved: set | None = None) -> list:
    """*Autofold*: split every face a move has warped out of its
    plane into planar pieces along fold edges.

    The warped face is triangulated (earcut over its Newell plane — robust for
    the gentle warps a vertex drag produces), then coplanar adjacent triangles
    are merged back so only the true folds remain: a quad with one lifted
    corner becomes exactly two triangles joined by one fold edge. Merging is
    confined to the pieces of the same source face, so a fold never dissolves
    into a coplanar neighbour across the original boundary. Returns the faces
    that were folded (the originals, already removed from the mesh).

    ``moved``: the position keys the move left its vertices at. A face whose
    corners are one run of moved and one run of unmoved ones folds as a
    strip between the two (#1294). Where the
    moved corners run along a curve (an arc's segments), the fold edges of
    its fan are soft, so the cone reads as one smooth surface, as the
    curve's own wall did; the fold between the flat piece and the cone
    stays a line."""
    from core.store_mesh import StoreMesh, _hypot
    if (type(mesh) is StoreMesh and not mesh.__dict__.get("_planned_points")
            and (hypot := _hypot()) is not None):
        result = mesh._store.fold_nonplanar(list(moved or ()), tolerance, hypot)
        if result is not None:
            return mesh._inherit_face_attrs(*result)

    store_scan = getattr(mesh, "nonplanar_faces", None)
    faces = store_scan(tolerance) if store_scan is not None else mesh.faces
    return kernel.object_replay.fold_objects(mesh, faces, tolerance, list(moved or ()))


def prune_collinear_orphan_edges(mesh) -> list:
    """Trim overlapping loose edges, carrying their flags onto uncovered stretches."""
    from core import scan_index
    from core.mesh import edge_flags, stamp_edge_flags
    scan = scan_index.scan(mesh) or scan_index.Scan(mesh)
    cuts = scan.native.orphan_pieces()
    removed = scan.edges(i for i, _ in cuts)
    curved = False
    for edge, (_, pieces) in zip(removed, cuts):
        flags = edge_flags(edge)
        mesh.remove_edge(edge)
        for start, end in pieces:
            a, b = QVector3D(*start), QVector3D(*end)
            va, vb = mesh.vertex_at(a), mesh.vertex_at(b)
            if va is not None and vb is not None and mesh.find_edge(va, vb) is not None:
                continue
            stamp_edge_flags(mesh.add_edge(a, b), flags)
            curved = curved or edge.curve is not None
    if curved:
        mesh.resplit_curves()
    return removed


def _mesh_is_flat(mesh) -> bool:
    """Whether every face lies on one common plane — a flat 2D drawing, where the
    aggressive partial-overlap cleanup is safe (in 3D, coplanar faces inside each
    other are legitimate)."""
    faces = mesh.faces
    if len(faces) < 2:
        return True
    from core import push_scans
    got = push_scans.mesh_is_flat(mesh, _PLANAR_TOLERANCE)
    if got is not None:
        return got
    return kernel.flat_faces(faces, _PLANAR_TOLERANCE)


#: Above this many faces the overlap heal is skipped entirely — see the
#: guard at the top of :func:`heal_overlapping_faces`.
_HEAL_FACE_CAP = 3000


#: Slack on the face boxes the heal filters pairs with: far past the
#: planar tolerance a "coplanar" point may sit off the plane by.
_BOX_PAD = 1e-2


def _heal_partners(N, A, LO, HI) -> dict:
    return kernel.heal_partners(N, A, LO, HI, _BOX_PAD)


def heal_overlapping_faces(mesh, coverage: float = 0.5, partial=None,
                           footprint=None) -> list:
    """Clean up coplanar face overlaps that draw/delete sequences can leave:

    1. **Redundant nested holes** — incrementally subdividing a face can punch
       overlapping holes (a hole inside a hole). Keep only the outermost ones.
    2. **A redundant mother** — a big enclosing face left on top of the smaller
       coplanar faces inside it (not merely filling its holes). When the inside
       faces cover most (> ``coverage``) of its area, that mother is spurious;
       remove it so the real subdivision stays. A ring (face with holes) whose
       inside faces only *fill its holes* is legitimate and kept.

    3. **Reversed faces** — a face auto-faced with the wrong winding (so it would
       push the wrong way) is flipped to match its plane. Given the edit's
       ``footprint`` (:func:`winding_footprint`), faces that were already
       there keep the way the user left them.

    The partial-overlap pass also removes a small face whose body lies in
    another's solid region — a partial overlap the auto-divide missed (e.g. a
    door-corner sliver). It's unsafe in 3D (stacked blocks, through-holes
    legitimately nest coplanar faces), so ``partial`` defaults to *auto*: on only
    when the whole model is one flat plane (a 2D drawing). ``True``/``False``
    force it.

    Returns the faces removed (the rebuilt/flipped ones don't count).
    """
    if len(mesh.faces) > _HEAL_FACE_CAP:
        # This heal exists for hand-drawing scale (the overlaps draw/delete
        # sequences leave); its mother-face pass is O(F²) and froze the UI
        # for minutes after exploding a 9k-face import (piscina.igz user
        # report — a single Delete took >120 s). At import scale the
        # cosmetic overlap cleanup is skipped; editing stays correct.
        return []
    flat = _mesh_is_flat(mesh)
    if partial is None:
        partial = flat

    # 0a. Prune duplicate division lines: an edge bounding no face that lies
    #     collinearly over another edge (the unwelded collinear-overlap that
    #     leaves a "doubled line" you can't merge away).
    prune_collinear_orphan_edges(mesh)

    # 0b. Weld T-junction seams so two faces share their dividing edge — erasing
    #     it then merges the walls instead of deleting one.
    resolve_tjunctions(mesh)

    # 0c. Flip any reversed face so coplanar faces face the same way — only
    #     meaningful on a flat drawing. In 3D one plane can legitimately carry
    #     opposite outwards (two solids back to back share it), so winding is
    #     the volumetric pass's job (``orient_outward`` below).
    if flat:
        orient_coplanar_faces(mesh, footprint)

    # 1. Dedupe nested holes by rebuilding the face with the outermost holes.
    from core.store_mesh import StoreMesh, FaceView, _hypot
    store = mesh._store if type(mesh) is StoreMesh and _hypot() is not None else None
    holed = (list(mesh.faces) if store is None else
             (FaceView(mesh, k) for k in store.multiple_hole_faces()))
    for face in holed:
        if len(face.holes) < 2:
            continue
        keep = _maximal_holes(face.holes)
        if len(keep) != len(face.holes):
            outer = [QVector3D(v) for v in face.vertices]
            mesh.remove_face(face)
            mesh.add_face(
                outer, [[QVector3D(v) for v in h] for h in keep] or None
            ).attrs = dict(face.attrs)

    import sys
    compensated = sys.version_info >= (3, 12)
    if store is not None:
        removed_keys, punches = store.heal_plan(
            coverage, bool(partial), _BOX_PAD, _hypot(), compensated)
        def face_at(key):
            return FaceView(mesh, key)
        removed = [face_at(k) for k in removed_keys]
        if removed_keys:
            store.remove_drawing_faces(removed_keys)
            mesh._dirty(len(removed_keys))
    else:
        faces = list(mesh.faces)
        removed_rows, punches = kernel.heal_objects(faces, coverage, bool(partial), _BOX_PAD, compensated)
        face_at = faces.__getitem__
        removed = [face_at(i) for i in removed_rows]
        for face in removed:
            mesh.remove_face(face)
    for ref, smalls in punches:
        _punch_overlaps(mesh, face_at(ref), [face_at(i) for i in smalls])
    if not flat:
        from core.orient import orient_outward
        orient_outward(mesh)
    return removed


def _punch_overlaps(mesh, g, smalls):
    outer = [QVector3D(v) for v in g.vertices]
    holes = [[QVector3D(v) for v in h] for h in g.holes]
    holes += [[QVector3D(v) for v in s.vertices] for s in smalls]
    mesh.remove_face(g)
    try:
        mesh.add_face(outer, holes).attrs = dict(g.attrs)
    except Exception:
        # Degenerate hole arrangement — fall back to the face as it was.
        mesh.add_face(outer, [[QVector3D(v) for v in h]
                              for h in g.holes] or None
                      ).attrs = dict(g.attrs)


#: The defects :func:`defects_near` counts, worst first — the order a
#: refusal names them in when a weld leaves more than one.
DEFECT_KINDS = ("duplicate_face", "overlapping_face", "pinched_face",
                "crossed_face", "flat_face", "zero_edge")


def defects_near(vertices, area_eps: float = 1e-10) -> dict:
    """Count defects incident to these vertices in landing-rejection priority order."""
    from core.store_mesh import StoreMesh, VertexView, _hypot
    from ingetrazo_core import MeshStore
    vertices = list(vertices)
    mesh = getattr(vertices[0], "_m", None) if vertices else None
    if type(mesh) is StoreMesh and all(type(v) is VertexView and v._m is mesh for v in vertices):
        try:
            values = mesh._store.landing_defects([v.key for v in vertices], area_eps, _hypot())
            return dict(zip(DEFECT_KINDS, values))
        except KeyError:
            pass  # A held view can resolve through a materialised object's key map.
    edges = {edge.key: edge for v in vertices for edge in v.edges}
    faces = {f.key: f for e in edges.values() for f in e.faces}
    def corner(v):
        return v.key, v.position.toTuple()
    values = MeshStore.object_landing_defects(
        [(corner(e.v0), corner(e.v1), [f.key for f in e.faces]) for e in edges.values()],
        [[[corner(v) for v in ring] for ring in (f.loop, *f.hole_loops)] for f in faces.values()],
        area_eps, _hypot())
    return dict(zip(DEFECT_KINDS, values))


def new_defects(before: dict, after: dict) -> list:
    """The defect kinds :func:`defects_near` counts more of ``after`` than
    ``before``, worst first."""
    from ingetrazo_core import MeshStore
    return MeshStore.increased_landing_defects([before.get(k, 0) for k in DEFECT_KINDS],
                                               [after.get(k, 0) for k in DEFECT_KINDS])
