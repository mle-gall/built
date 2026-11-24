# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Edit operations that turn drawn segments into history commands.

This is the bridge between the pure geometry planner in :mod:`core.topology`
and the undo/redo commands in :mod:`core.history`. Tools hand it the raw
segments the user drew; it returns a single reversible command that:

- splits existing edges the new segments cross (the classic auto-split),
- breaks each new segment at those crossings,
- welds coincident edges (via ``AddEdgeCommand``'s merge), and
- optionally auto-faces any planar cycle the new sub-edges close.

Segments are processed in order against a running simulation of the scene's
edge list, so a batch (e.g. a rectangle's four edges) splits correctly even
when later segments cross edges created by earlier ones.

Kept in its own module to avoid an import cycle: ``history`` imports
``topology``, so the command-building glue can't live in either of them.
"""
from __future__ import annotations
from core.mesh import element_identity

from typing import Iterable, Optional, Sequence

from ingetrazo_core import Vec3 as QVector3D, object_drawing, polygon_newell as _newell

_order_chain = object_drawing.order_chain

from core.geometry import Edge, Face
from core.history import (
    AddEdgeCommand,
    AddFaceCommand,
    Command,
    CompoundCommand,
    DeleteEdgesCommand,
    DeleteFaceCommand,
    SnapshotCompound,
)
from core.topology import (
    _point_in_face_solid,
    face_exists,
    find_chord_split,
    find_cycles_through,
    find_duplicate_edge,
    is_planar,
    plan_edge_split,
    split_edge_in_faces,
)

Segment = tuple[QVector3D, QVector3D]


def touching_segments(segments: Sequence[Segment], edges: Iterable[Edge],
                      faces: Iterable[Face]) -> list[Segment]:
    """Copy only needs to plan segments that touch geometry OUTSIDE the copy.

    Test real contacts after the box filter: disjoint pieces can have
    overlapping boxes. Never rediscover intersections within supplied
    topology; imports may intentionally contain long edges over facets.
    """
    if not segments:
        return []
    from core.topology import (_point_in_face_solid, _point_on_seg_incl,
                               segment_intersection)
    edges, faces = list(edges), list(faces)
    near = _Nearby(edges, faces)
    out = []
    for a, b in segments:
        if any(segment_intersection(a, b, e.a, e.b) is not None
               or _point_on_seg_incl(a, e.a, e.b)
               or _point_on_seg_incl(e.a, a, b)
               for e in near.edges(edges, a, b)):
            out.append((a, b))
            continue
        for f in near.faces(faces, a, b):
            normal = f.normal()
            origin = f.vertices[0]
            if (normal.lengthSquared() > 0.5
                    and abs(QVector3D.dotProduct(a - origin, normal)) < 1e-4
                    and abs(QVector3D.dotProduct(b - origin, normal)) < 1e-4
                    and any(_point_in_face_solid(f, p)
                            for p in (a, b, (a + b) * 0.5))):
                out.append((a, b))
                break
    return out


def plan_edge_commands(
    scene,
    segments: Sequence[Segment],
    detect_faces: bool = True,
    eye=None,
    *,
    close_cycles: bool = True,
    soft=None, tag=None,
) -> list[Command]:
    """Build the ordered command list to add ``segments`` to ``scene``.

    Mirrors what each emitted command will do to the scene in a local
    ``simulated`` edge list, so planning and execution stay in lock-step.
    Returns a flat list (callers wrap it, possibly alongside their own
    commands such as a tool-managed face, in a single ``CompoundCommand``).
    ``close_cycles=False`` keeps chord splits but never infers new faces
    from cycles, for tools that carry existing topology.
    """
    from core import scan_index
    from core.topology import KeyMemo
    sc = scan_index.scan(scene.mesh)
    if sc is not None:
        # The mesh as arrays (core.scan_index): the lists below are the
        # scan's, edited as the plan goes, and their searches ask it.
        simulated = _ScanEdges(sc)
        faces_snapshot = _ScanFaces(sc)
        near = _ScanNearby()
    else:
        simulated = _PlanEdges(scene.edges)
        faces_snapshot = list(scene.faces)
        near = _Nearby(simulated, faces_snapshot)
    near.key = KeyMemo()            # nothing moves while a plan is made
    simulated.key = near.key

    return object_drawing.plan_segments(simulated, faces_snapshot, near, segments,
                                        sc is not None, detect_faces, close_cycles,
                                        eye, soft, tag)


class _PlanEdges(list):
    """A plan's edge list, with the graph the cycle search walks kept
    beside it: built the first time a cycle is asked for, and following
    every ``append`` and ``remove`` after that. A plan only ever changes
    the list through those two."""

    key = None
    _graph = None

    def graph(self):
        if self._graph is None:
            from core.topology import CycleGraph, _key
            self._graph = CycleGraph(self, self.key or _key)
        return self._graph

    def append(self, edge) -> None:
        list.append(self, edge)
        if self._graph is not None:
            self._graph.add(edge)

    def remove(self, edge) -> None:
        list.remove(self, edge)
        if self._graph is not None and not self._graph.remove(edge):
            self._graph = None      # built again at the next question


class _Nearby:
    """Box filter for the planner's searches. Every edge and face is boxed
    once per plan, in one NumPy array (the plan's own simulated pieces are
    boxed on first sight); a search then gets, in their original order, only
    those whose box, grown by a margin far beyond the 0.1 mm welding
    tolerance, reaches the segment. Anything that can cross, contain or
    duplicate the segment is kept, so the plan is the one the full lists
    give (tests/test_planner_box_filter.py)."""

    def __init__(self, edges, faces) -> None:
        import numpy as np
        self._np = np
        self._items: list = []
        self._row: dict = {}
        rows = []
        for e in edges:
            self._row[element_identity(e)] = len(self._items)
            self._items.append(e)
            a, b = e.a, e.b
            ax, ay, az, bx, by, bz = a.x(), a.y(), a.z(), b.x(), b.y(), b.z()
            rows.append((min(ax, bx), min(ay, by), min(az, bz),
                         max(ax, bx), max(ay, by), max(az, bz)))
        for f in faces:
            self._row[element_identity(f)] = len(self._items)
            self._items.append(f)
            rows.append(self._points_box(f.vertices))
        self._arr = (np.asarray(rows, dtype=np.float64) if rows
                     else np.zeros((0, 6)))
        self._extra: dict = {}

    @staticmethod
    def _points_box(pts):
        xs = [p.x() for p in pts]
        ys = [p.y() for p in pts]
        zs = [p.z() for p in pts]
        if not xs:
            return (0.0,) * 6
        return (min(xs), min(ys), min(zs), max(xs), max(ys), max(zs))

    def _box(self, item):
        hit = self._extra.get(element_identity(item))
        if hit is not None and hit[0] is item:
            return hit[1]
        pts = item.vertices if hasattr(item, "vertices") else (item.a, item.b)
        box = self._points_box(pts)
        self._extra[element_identity(item)] = (item, box)
        return box

    @staticmethod
    def _window(pts):
        xs = [p.x() for p in pts]
        ys = [p.y() for p in pts]
        zs = [p.z() for p in pts]
        lo = (min(xs), min(ys), min(zs))
        hi = (max(xs), max(ys), max(zs))
        # 1 cm, plus a relative term for coordinates far from the origin
        # (float32 positions, segment_intersection's relative tolerance).
        m = 0.01 + 1e-6 * max(abs(c) for c in lo + hi)
        return (lo[0] - m, lo[1] - m, lo[2] - m,
                hi[0] + m, hi[1] + m, hi[2] + m)

    def _pick(self, items, pts):
        np = self._np
        w = self._window(pts)
        arr = self._arr
        mask = ((arr[:, 3] >= w[0]) & (arr[:, 0] <= w[3])
                & (arr[:, 4] >= w[1]) & (arr[:, 1] <= w[4])
                & (arr[:, 5] >= w[2]) & (arr[:, 2] <= w[5]))
        known = self._items
        # Boxed at the start (kept alive in ``known``, so an id is never
        # reused): the mask says. The plan's own new pieces are boxed here.
        # The set work runs in C; only the hits and the new pieces reach
        # Python, and the result keeps the order of ``items``.
        keep = {element_identity(known[r]) for r in np.flatnonzero(mask)}
        ids = list(map(element_identity, items))
        new = set(ids).difference(self._row)
        if new:
            for it in items:
                if element_identity(it) in new:
                    x0, y0, z0, x1, y1, z1 = self._box(it)
                    if (x1 >= w[0] and x0 <= w[3] and y1 >= w[1]
                            and y0 <= w[4] and z1 >= w[2] and z0 <= w[5]):
                        keep.add(element_identity(it))
        if not keep:
            return []
        return [it for it, i in zip(items, ids) if i in keep]

    def edges(self, edges, a, b):
        return self._pick(edges, (a, b))

    def faces(self, faces, a, b):
        return self._pick(faces, (a, b))

    def faces_around(self, faces, pts):
        return self._pick(faces, list(pts))


class _ScanList:
    """A plan's running copy of the mesh's edge or face list over a mesh
    scan (``core.scan_index``): the list at the scan's revision, less what
    the plan removed, with what it replaced in place and what it appended
    after — the operations :func:`plan_edge_commands` makes on its
    ``simulated`` / ``faces_snapshot`` lists — without copying a list of
    half a million elements per click. :meth:`pick` is ``_Nearby._pick``:
    the items whose box (``_Nearby._points_box``, the same floats) meets
    the window, in list order."""

    _kind = ""

    def __init__(self, sc) -> None:
        self._sc = sc
        self._got: dict = {}          # scan index -> element handed out
        self._at: dict = {}           # element_identity -> scan index
        self._dead: set = set()       # scan indices removed
        self._repl: dict = {}         # scan index -> replacement item
        self._extra: list = []        # appended items, in order
        self._boxes: dict = {}

    def _elements(self, idx) -> list:
        new = [i for i in idx if i not in self._got]
        if new:
            for i, e in zip(new, self._elements_of(new)):
                self._got[i] = e
                self._at[element_identity(e)] = i
        return [self._got[i] for i in idx]

    def _elements_of(self, idx):
        raise NotImplementedError

    def _scan_hits(self, w) -> list:
        raise NotImplementedError

    def _base(self, item):
        """The scan index ``item`` stands at, or None (not a scan element)."""
        i = self._at.get(element_identity(item))
        if i is not None and self._got[i] == item:
            return i
        return None

    def _box(self, item):
        hit = self._boxes.get(item)
        if hit is not None:
            return hit
        pts = item.vertices if hasattr(item, "vertices") else (item.a, item.b)
        box = _Nearby._points_box(pts)
        self._boxes[item] = box
        return box

    @staticmethod
    def _meets(box, w) -> bool:
        x0, y0, z0, x1, y1, z1 = box
        return (x1 >= w[0] and x0 <= w[3] and y1 >= w[1]
                and y0 <= w[4] and z1 >= w[2] and z0 <= w[5])

    def pick(self, pts) -> list:
        w = _Nearby._window(pts)
        idx = [i for i in self._scan_hits(w)
               if i not in self._dead and i not in self._repl]
        keyed = list(zip(idx, self._elements(idx)))
        if self._repl:
            # A replacement stands where the element it replaced stood,
            # boxed afresh (``_Nearby._box`` of a new item).
            keyed += [(i, r) for i, r in self._repl.items()
                      if i not in self._dead and self._meets(self._box(r), w)]
            keyed.sort(key=lambda t: t[0])
        out = [x for _i, x in keyed]
        out.extend(x for x in self._extra if self._meets(self._box(x), w))
        return out

    # ---- the list operations the planner makes --------------------------
    def __contains__(self, item) -> bool:
        i = self._base(item)
        if i is not None and i not in self._dead and i not in self._repl:
            return True
        if any(r == item for j, r in self._repl.items() if j not in self._dead):
            return True
        return item in self._extra

    def remove(self, item) -> None:
        i = self._base(item)
        if i is not None and i not in self._dead and i not in self._repl:
            self._dead.add(i)
            return
        for j, r in self._repl.items():
            if r == item and j not in self._dead:
                self._dead.add(j)
                return
        self._extra.remove(item)

    def append(self, item) -> None:
        self._extra.append(item)

    def index(self, item):
        """A position token for ``__setitem__`` (the planner replaces what
        it found, ``l[l.index(f)] = g``)."""
        i = self._base(item)
        if i is not None and i not in self._dead and i not in self._repl:
            return ("base", i)
        for j, r in self._repl.items():
            if r == item and j not in self._dead:
                return ("base", j)
        return ("extra", self._extra.index(item))

    def __setitem__(self, where, item) -> None:
        kind, i = where
        if kind == "base":
            self._repl[i] = item
        else:
            self._extra[i] = item


class _ScanEdges(_ScanList):
    """``simulated``: the plan's edge list over the scan."""

    def _elements_of(self, idx):
        return self._sc.edges(idx)

    def _scan_hits(self, w) -> list:
        return self._sc.native.edges_meeting(w[:3], w[3:])

    def split_plan(self, a, b, *, whole=False):
        from core.store_mesh import _hypot
        import sys
        hypot = _hypot() or "libstdc++"
        window = _Nearby._window((a, b))
        indices = (range(self._sc.native.counts()[1]) if whole else
                   [i for i in self._scan_hits(window) if i not in self._dead])
        extras = [e for e in self._extra if self._meets(self._box(e), window)]
        hits, cuts = self._sc.native.split_plan(
            a.toTuple(), b.toTuple(), list(indices), [(e.a.toTuple(), e.b.toTuple()) for e in extras],
            hypot, sys.version_info >= (3, 12))
        chain = [a, *(QVector3D(*p) for p in hits), b]
        def edge(i):
            return self._elements([indices[i]])[0] if i < len(indices) else extras[i - len(indices)]
        return list(zip(chain, chain[1:])), {edge(i): QVector3D(*p) for i, p in cuts}

    def drawing_cycles(self, a, b, limit, both):
        extra = [(e.a.toTuple(), e.b.toTuple()) for e in self._extra]
        cycles = self._sc.native.drawing_cycles(
            a.toTuple(), b.toTuple(), list(self._dead), extra, max(0, limit), both)
        return [[QVector3D(*p) for p in cycle] for cycle in cycles]

    def cycle_adjacency(self, ka, kb):
        """``topology._cycle_adjacency`` of this list, built per node as the
        search asks (``_LazyAdjacency``)."""
        return _LazyAdjacency(self, ka, kb)


class _ScanFaces(_ScanList):
    """``faces_snapshot``: the plan's face list over the scan."""

    def _elements_of(self, idx):
        return self._sc.faces(idx)

    def _scan_hits(self, w) -> list:
        return self._sc.native.faces_meeting(w[:3], w[3:])


class _LazyAdjacency:
    """``adj`` of ``topology._cycle_adjacency(edges, ka, kb)`` for a
    :class:`_ScanEdges` list, made node by node: a node's list holds, in
    list order, every edge with an end of that key (not degenerate, not a
    copy of ``ka``–``kb``) — the scan's edges that may end there (the scan
    files them by key cell), less those the plan removed, then the plan's
    own pieces. Each entry is ``(other end's key, its position)``, the
    position the same float32 value the edge's end holds. A breadth-first
    search over it visits what the full adjacency would let it visit, in
    the same order, so it finds the same cycle."""

    def __init__(self, edges: _ScanEdges, ka, kb) -> None:
        from core.topology import _key
        self._edges = edges
        self._pair = {ka, kb}
        self._lists: dict = {}
        self._extra: dict = {}
        for e in edges._extra:
            ea, eb = _key(e.a), _key(e.b)
            if ea == eb or {ea, eb} == self._pair:
                continue
            self._extra.setdefault(ea, []).append((eb, e.b))
            self._extra.setdefault(eb, []).append((ea, e.a))
        # Replaced edges are not something the planner does to edges.
        assert not edges._repl

    def _list(self, u) -> list:
        hit = self._lists.get(u)
        if hit is not None:
            return hit
        sc = self._edges._sc
        dead = self._edges._dead
        out = []
        for e, va, vb in sc.edges_ending_at(u):
            if e in dead:
                continue
            ea, eb = sc.tkey(va), sc.tkey(vb)
            if ea == eb or {ea, eb} == self._pair:
                continue
            if ea == u:
                out.append((eb, sc.vec(vb)))
            elif eb == u:
                out.append((ea, sc.vec(va)))
        out.extend(self._extra.get(u, ()))
        self._lists[u] = out
        return out

    def __contains__(self, u) -> bool:
        return bool(self._list(u))

    def __getitem__(self, u) -> list:
        lst = self._list(u)
        if not lst:
            raise KeyError(u)
        return lst

    def get(self, u, default=()):
        lst = self._list(u)
        return lst if lst else default


class _ScanNearby:
    """``_Nearby`` for the scan's lists: their own :meth:`_ScanList.pick`."""

    key = None

    @staticmethod
    def edges(edges, a, b):
        return edges.pick((a, b))

    @staticmethod
    def faces(faces, a, b):
        return faces.pick((a, b))

    @staticmethod
    def faces_around(faces, pts):
        return faces.pick(list(pts))


def _scene_has_curves(scene) -> bool:
    from core import scan_index
    sc = scan_index.scan(scene.mesh)
    if sc is not None:
        return sc.native.any_curve()
    return any(getattr(e, "curve", None) is not None for e in scene.edges)


def _append_flat_curve_rebuild(scene, commands, points, *, close_cycles=True) -> None:
    """Append the deterministic planar-arrangement rebuild after a draw when
    the cycle planner alone cannot produce the expected regions:

    - Whole-flat drawing WITH curves → full rebuild (every minimal region gets
      a face; a square over a circle splits into three areas).
    - Whole-flat drawing, straight edges only, faces present → SCOPED rebuild
      with coverage semantics (two overlapping rectangles split into three
      regions; regions nobody covered stay empty — no resurrecting faces the
      user deleted).
    - 3D scene whose drawing plane carries curve edges → SCOPED rebuild on
      just that plane (straight-only 3D drawing keeps its proven path).

    Dangling edges survive the rebuild (spur pruning only affects face
    tracing), so half-drawn chains are safe."""
    from core.history import RebuildPlanarFacesCommand, RebuildPlaneFacesCommand

    if any(isinstance(c, (RebuildPlanarFacesCommand, RebuildPlaneFacesCommand))
           for c in commands):
        return
    from core.arrangement import coplanar_plane

    has_curves = _scene_has_curves(scene)
    if _whole_drawing_plane(scene, points) is not None:
        if has_curves and close_cycles:
            # Whole-flat drawing with curves: the full rebuild (every minimal
            # region gets a face — how a lone circle yields its disc).
            commands.append(RebuildPlanarFacesCommand())
        elif 0 < len(scene.mesh.faces) <= 400:
            # Apply the populated-plane rebuild limit to flat imports too.
            # Whole-flat, straight edges only, faces present: the SCOPED
            # rebuild with coverage semantics. Two overlapping rectangles must
            # split into three regions (the classic behaviour) — the planner leaves
            # the first rectangle whole over the lens (rect.igz report) — but
            # regions nobody covered stay empty (no resurrecting faces the
            # user deleted).
            normal = scene.mesh.faces[0].normal()
            origin = scene.mesh.faces[0].vertices[0]
            commands.append(RebuildPlaneFacesCommand(origin, normal))
        return
    if not has_curves:
        return
    plane = coplanar_plane(list(points))
    if plane is None:
        return
    origin, normal = plane
    tol = 1e-4

    def on_plane(p):
        return abs(QVector3D.dotProduct(p - origin, normal)) < tol

    from core import scan_index
    sc = scan_index.scan(scene.mesh)
    if sc is None:
        edges = scene.mesh.edges
    else:
        edges = sc.edges(sc.native.edges_on_plane(
            (origin.x(), origin.y(), origin.z()),
            (normal.x(), normal.y(), normal.z()), tol, True))
    if any(e.curve is not None and on_plane(e.a) and on_plane(e.b)
           for e in edges):
        commands.append(RebuildPlaneFacesCommand(origin, normal))


def _whole_drawing_plane(scene, points):
    """``coplanar_plane(every vertex of the mesh + points)``: whether the
    whole drawing is flat. Its answer hangs on the first few points (they
    fix the plane) and then on whether ANY point leaves it, so with a mesh
    scan the first points are walked as the function walks them and the
    rest are judged in one array pass — a point clearly off the plane, by
    more than the float32 error of the test, decides; the doubtful ones go
    through the test itself. Without a scan, the call on the full list."""
    from core.arrangement import coplanar_plane
    from core import scan_index
    sc = scan_index.scan(scene.mesh)
    if sc is None:
        verts = [v.position for v in scene.mesh.vertices] + list(points)
        return coplanar_plane(verts)
    from core._native import qt_hypot
    source = scene.mesh._store if sc.store_backed else sc.native
    plane = source.drawing_plane([p.toTuple() for p in points], qt_hypot())
    return None if plane is None else tuple(QVector3D(*p) for p in plane)


def _append_face_plane_rebuild(scene, commands, points) -> None:
    """Append the scoped plane rebuild when straight edges land on a plane
    that already carries faces, in a scene the whole-flat gate can't cover.

    The naive cycle planner mishandles a chord that touches a hole boundary
    (a line across a slab top grazing an inner rectangle's edge): instead of
    subdividing the mother it stacks a duplicate, sometimes flipped, region
    face on top — and erasing any of those edges later cascade-deletes the
    whole plane instead of merging (aa.igz plaza report, 2026-07-12). The
    deterministic arrangement with coverage semantics is the machinery that
    already solves this for curves; a populated plane deserves it too."""
    from core.history import RebuildPlanarFacesCommand, RebuildPlaneFacesCommand

    if any(isinstance(c, (RebuildPlanarFacesCommand, RebuildPlaneFacesCommand))
           for c in commands):
        return
    tol = 1e-4
    from core import scan_index
    sc = scan_index.scan(scene.mesh)
    from core._native import qt_hypot
    if sc is not None:
        source = scene.mesh._store if sc.store_backed else sc.native
        plane = source.rebuild_plane([p.toTuple() for p in points], 400, qt_hypot())
        if plane is not None:
            commands.append(RebuildPlaneFacesCommand(*(QVector3D(*p) for p in plane)))
        return
    faces = scene.mesh.faces
    for f in faces:
        origin = f.vertices[0]
        normal = f.normal()
        # A degenerate normal is a WILDCARD here: dot(anything, zero) is 0,
        # so such a face would claim every line drawn anywhere in the mesh
        # and hand its "plane" to the rebuild below. Face.normal() no longer
        # produces one, but nothing downstream should depend on that.
        if normal.lengthSquared() < 0.5:
            continue
        if all(abs(QVector3D.dotProduct(p - origin, normal)) < tol
               for p in points):
            # Reference-scale guard: rebuilding a plane that carries hundreds
            # of faces (a triangulated import living loose in the mesh) runs
            # the arrangement over all of them — minutes, reads as a hang.
            # Hand-drawn planes stay far below this; past it, keep the naive
            # path (imports should live in a Group anyway).
            others = scene.mesh.faces
            on_plane = sum(
                1 for g in others
                if abs(QVector3D.dotProduct(g.vertices[0] - origin,
                                            normal)) < tol
                and abs(QVector3D.dotProduct(g.normal(), normal)) > 0.999)
            if on_plane <= 400:
                commands.append(RebuildPlaneFacesCommand(origin, normal))
            return


def _append_active_tag(scene, commands) -> None:
    """Tag-as-you-draw: when the BIM active class is set, append the step that
    stamps it on the plan's own drawn faces. Inserted BEFORE the plane
    rebuilds, so a rebuild that replaces the drawn face inherits the tag
    per-region (A.3) instead of losing the reference."""
    tag = getattr(scene, "active_ifc", None)
    if not tag:
        return
    from core.history import AutoTagDrawnFacesCommand
    face_cmds = [c for c in commands
                 if isinstance(c, AddFaceCommand) and c.auto]
    if face_cmds:
        commands.append(AutoTagDrawnFacesCommand(face_cmds, tag))


def face_up_or_toward(loop, eye=None):
    """The winding a brand-new face should take (the usual rule for a face
    with no neighbour to agree with): a horizontal one shows its FRONT
    upwards, any other one faces the ``eye`` that drew it. A loop closed by
    hand with Line, or by Offset, came out either way — the order the cycle
    was traced in — and Marco read the blue back as «cara invertida»
    (2026-09-21, tests 15 and 19 of the bench). Returns the loop, reversed
    when needed; untouched when nothing decides (no eye and a vertical
    face, or a degenerate loop)."""
    pts = list(loop)
    if len(pts) < 3:
        return pts
    n = _newell(pts)
    if n.length() < 1e-12:
        return pts
    if abs(n.z()) > 0.5 * n.length():
        want = n.z() > 0
    elif eye is not None:
        c = QVector3D(0, 0, 0)
        for q in pts:
            c += q
        c /= float(len(pts))
        want = QVector3D.dotProduct(n, QVector3D(eye) - c) > 0
    else:
        return pts
    return pts if want else [pts[0]] + pts[:0:-1]


def build_add_edge(scene, a: QVector3D, b: QVector3D, detect_faces: bool = True,
                   eye=None, *, soft=None, tag=None) -> Command:
    """Single-segment convenience: one drawn edge → one atomic command."""
    commands = plan_edge_commands(scene, [(a, b)], detect_faces=detect_faces,
                                  eye=eye, soft=soft, tag=tag)
    _append_active_tag(scene, commands)
    _append_flat_curve_rebuild(scene, commands, [a, b])
    if detect_faces:
        _append_face_plane_rebuild(scene, commands, [a, b])
    # When curves exist, even a single added edge can break a circle into
    # contours (a tangent line landing on a curve vertex splits it),
    # so it must go through SnapshotCompound, which runs the contour re-split —
    # and whose undo restores the reunited curve.
    if len(commands) == 1 and soft is None and tag is None and not _scene_has_curves(scene):
        return commands[0]
    # Splits/welds/hole-punches don't compose into a clean per-op inverse — undo
    # via one snapshot so it reverses exactly (no orphan edges/vertices left).
    return SnapshotCompound(commands)


def build_add_edges(
    scene,
    segments: Sequence[Segment],
    detect_faces: bool = True,
    extra: Iterable[Command] = (),
    eye=None,
    plane_rebuild: bool = False,
    *,
    preserve_faces: bool = False,
) -> Command:
    """Batch convenience: many drawn edges (+ optional ``extra`` commands such
    as a tool-managed face) → one atomic command. ``eye`` (the camera's
    position) decides which way a new vertical face looks.

    ``plane_rebuild``: a tool that owns its face (the Rectangle) asks for
    the plane it lands on to be rebuilt, as a drawn Line has been since
    July — drawn over a ring on walls it stacked a full face on the ring
    instead of filling the hole (#538).

    ``preserve_faces``: copied geometry supplies its own faces. Split faces
    the edges cross, but do not face empty cycles or globally heal the mesh.
    Rebuild coverage only for cuts or an explicitly requested plane rebuild;
    callers must keep isolated copies out of the planner (#1087).
    """
    commands = plan_edge_commands(scene, segments, detect_faces=detect_faces,
                                  eye=eye, close_cycles=not preserve_faces)
    commands.extend(extra)
    _append_active_tag(scene, commands)
    points = [p for seg in segments for p in seg]
    cuts = any(isinstance(c, (DeleteFaceCommand, DeleteEdgesCommand)) for c in commands)
    if not preserve_faces or cuts or plane_rebuild:
        _append_flat_curve_rebuild(scene, commands, points,
                                   close_cycles=not preserve_faces)
        if plane_rebuild:
            _append_face_plane_rebuild(scene, commands, points)
    if len(commands) == 1 and not _scene_has_curves(scene):
        return commands[0]
    return SnapshotCompound(commands, heal=not preserve_faces)


def divide_edges(mesh, edges, n: int) -> int:
    """Divide (issue #63, @pacaeiro): split each selected edge —
    or, for an edge of a curve, the whole curve — into ``n`` pieces of
    equal length. A straight edge gets n−1 new vertices. A curve (arc,
    circle) is measured along its chain, cut where the k/n marks fall, and
    comes out as ``n`` INDEPENDENT arcs — each piece its own curve, each
    selectable on its own, the classic behaviour (a circle divided in
    four is four quarter arcs; Marco, 2026-09-21).
    A mark that lands on an existing facet vertex cuts nothing there but
    still parts the curve. Faces the edges bound take the new vertices in
    their loops (``Mesh.split_edge``). Returns the number of cuts made."""
    n = int(n)
    if n < 2:
        return 0
    from core import mesh as mesh_module
    from core.store_mesh import StoreMesh, _hypot
    import sys
    if type(mesh) is StoreMesh and (hypot := _hypot()) is not None:
        (cuts, next_id), change = mesh._store.divide_edges(
            [e.key for e in edges], n, mesh_module._CURVE_COUNTER, hypot,
            sys.version_info >= (3, 12))
        mesh_module._CURVE_COUNTER = next_id
        mesh._dirty(3 * cuts)
        if change.modified["edge"]:
            mesh._chunk_dirty = True
        return cuts
    return object_drawing.divide_edges(mesh, edges, n)


#: Why Join Segments leaves a point where it is, for its preview: the
#: points it takes away ("join") and the ones on a straight line that it
#: keeps all the same, each for its reason. A plain corner is not listed:
#: only a kink close enough to straight to be mistaken for none.
JOIN_REASONS = ("join", "kink", "branch", "flags", "curve")

def join_edges(mesh, edges, verdicts: Optional[dict] = None) -> list:
    """Join Segments, the inverse of Divide: every vertex where two of
    ``edges`` meet in a straight line, with nothing else attached to it,
    goes, and the two segments become one line. A long line drawn in
    pieces, or one left cut by a divide or by an erased crossing, comes out
    clean, its faces (if it bounds any) losing the needless corner.

    A vertex stays when a third edge meets it, when the two segments are
    not collinear, bound different faces, carry different flags (soft,
    hidden, layer), or belong to a curve: an arc's facets are not
    segments to merge. Returns the edges the selection is made of
    afterwards, the joined lines in place of their pieces.

    ``verdicts``, when given, is filled with what happened to each point
    on the way, ``{vertex: reason}`` (see :data:`JOIN_REASONS`): what
    :func:`join_preview` shows."""
    live = [e for e in edges if e in mesh.edges]
    from core.store_mesh import StoreMesh, EdgeView, VertexView, _hypot
    import sys
    if type(mesh) is StoreMesh and (hypot := _hypot()) is not None:
        (keys, reasons, joins), _ = mesh._store.join_edges(
            [e.key for e in live], hypot, sys.version_info >= (3, 12))
        mesh._dirty(3 * joins)
        if verdicts is not None:
            verdicts.update((VertexView(mesh, k), why) for k, why in reasons)
        return [EdgeView(mesh, k) for k in keys]
    return object_drawing.join_edges(mesh, live, verdicts)


def join_preview(mesh, edges) -> dict:
    """What Join Segments would do to ``edges``, without doing it:
    ``{vertex: reason}`` for the points it would take away and the ones on
    a straight line it would keep (see :data:`JOIN_REASONS`). A dry run of
    :func:`join_edges` itself, so the preview never disagrees with the
    command; the mesh is put back as it was, the same objects included."""
    verdicts: dict = {}
    snap = mesh.capture_state()
    try:
        join_edges(mesh, edges, verdicts)
    finally:
        mesh.restore_state(snap)
    return verdicts
