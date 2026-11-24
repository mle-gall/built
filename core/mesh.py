# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Shared-vertex, non-manifold connectivity mesh.

Level B+C of the topology-engine migration (see
``docs/halfedge-migration-plan.md``). Unlike the legacy ``core.geometry`` model
— where every ``Edge``/``Face`` stores its own *copies* of points and
connectivity is rediscovered by matching rounded positions — here a
:class:`Vertex` is a first-class **shared** object:

- an :class:`Edge` references two vertices and knows its incident faces (a
  *radial* list that may hold more than two — non-manifold is fine: three faces
  meet where two walls and a floor join);
- a :class:`Face` is bounded by loops of shared vertices, with optional holes;
- the :class:`Mesh` welds coincident positions to a single vertex on insert
  ("sticky" geometry) and maintains the incidence as geometry is added/removed.

This is the classic surface-modeller model. Moving a vertex moves every edge and face that
references it for free — no position-matching, no float tolerance.

Built in parallel with the legacy model; the app is migrated onto it
incrementally (phase M2) behind the ``Command`` facade. Nothing here is wired
into the running app yet.
"""
from __future__ import annotations

from contextvars import ContextVar
from typing import Iterable, Optional
from itertools import count

from ingetrazo_core import Vec3 as QVector3D


# Weld tolerance — two positions within this many decimals (~0.1 mm at metric
# scale) are the same vertex. Same value as legacy ``core.topology``.
_KEY_DECIMALS = 4

# Monotonic id for drawn curves (circle/arc). Segments of one curve share it so
# selection groups them as one curve. Session-scoped; ``.igz`` load bumps it
# past any stored ids so new curves stay unique.
_CURVE_COUNTER = 1

# Separate namespaces keep keys unambiguous in scene-wide selections as well
# as within a mesh. Consumers must treat the resulting int as opaque.
_KEY_NAMESPACES = count(1)
_STANDALONE_KEYS = count(-1, -1)


# Distance tolerance for the stitch pass (a point this close to an edge counts
# as on it). Matches the weld key resolution.
_STITCH_TOL = 1e-4


_KEY_SCALE = 10.0 ** _KEY_DECIMALS

#: ``Mesh.__dict__`` entries of a store-backed or materialised mesh that
#: belong to the store (``core.store_mesh.STORE_SIDE``): never deep-copied.
_STORE_SIDE = frozenset(("_store", "_objmap", "_attr_table", "_attr_own",
                         "_bulk", "_py_base", "_pos_cache",
                         "_face_cache", "_frame_parent", "_tool_frame_cache",
                         "_planned_points"))


def _store_on() -> bool:
    """Whether new meshes go to the Rust store."""
    from core import store_mesh
    return store_mesh.enabled()


def _key(p: QVector3D) -> tuple[int, int, int]:
    # Integer grid cells (same partition as rounding to _KEY_DECIMALS):
    # neighbouring cells are ±1, so the boundary probes in _lookup and
    # weld_coincident need no float rounding — the rounding calls dominated
    # large imports.
    x, y, z = p.toTuple()
    return (round(x * _KEY_SCALE),
            round(y * _KEY_SCALE),
            round(z * _KEY_SCALE))


def _weld_candidates(keys):
    """Which distinct cells touch another cell in the batch (26 neighbours).

    Compress each axis, keeping a gap of two for every gap larger than one:
    adjacency is unchanged, but the packed keys fit in int64 for far larger
    coordinate ranges. Two padding cells prevent carries between axes from
    looking like neighbours. Each positive offset finds both ends of a pair,
    so thirteen integer searches replace twenty-six three-axis searches.
    """
    import numpy as np

    candidate = np.zeros(len(keys), dtype=bool)
    if len(keys) < 2:
        return candidate
    codes, spans = [], []
    for axis in keys.T:
        values, inverse = np.unique(axis, return_inverse=True)
        steps = np.ones(len(values), dtype=np.int64)
        steps[0] = 0
        steps[1:] += np.diff(values) != 1
        ranks = steps.cumsum()
        codes.append(ranks[inverse])
        spans.append(int(ranks[-1]) + 3)
    nx, ny, nz = spans
    if nx * ny * nz > np.iinfo(np.int64).max:
        # An exceptionally large batch: use unbounded integers, never wrap
        # a packed key and change the set (and thus the vertex order).
        codes = [c.astype(object) for c in codes]
    packed = (codes[0] * ny + codes[1]) * nz + codes[2]
    order = np.argsort(packed)
    ordered = packed[order]
    for dx in (0, 1):
        for dy in (-1, 0, 1):
            for dz in (-1, 0, 1):
                offset = (dx * ny + dy) * nz + dz
                if offset <= 0:
                    continue
                indices = np.searchsorted(ordered, ordered + offset)
                src = np.flatnonzero(indices < len(ordered))
                dst = indices[src]
                hit = ordered[dst] == ordered[src] + offset
                candidate[order[src[hit]]] = True
                candidate[order[dst[hit]]] = True
    return candidate


class Vertex:
    """A shared point in the mesh.

    Coincident positions weld to one ``Vertex`` (the :class:`Mesh` registry
    guarantees this), so moving it moves every edge and face that references it.
    ``edges`` holds the incident edges, maintained by the mesh.
    """

    __slots__ = ("position", "edges", "key")

    def position_tuple(self):
        return self.position.toTuple()

    def __init__(self, position: QVector3D, *, key: int | None = None) -> None:
        self.key = next(_STANDALONE_KEYS) if key is None else key
        self.position = QVector3D(position)
        self.edges: set[Edge] = set()

    def faces(self) -> set[Face]:
        """Faces touching this vertex (via its incident edges)."""
        out: set[Face] = set()
        for e in self.edges:
            out.update(e.faces)
        return out

    def __repr__(self) -> str:  # pragma: no cover - debug aid
        p = self.position
        return f"Vertex({p.x():.3f}, {p.y():.3f}, {p.z():.3f})"


class Edge:
    """A segment between two shared vertices.

    ``faces`` is the *radial* list of incident faces — 0, 1, 2, or more. More
    than two is non-manifold and fully supported (it is the common case in
    architecture). Maintained by the mesh.
    """

    __slots__ = ("key", "v0", "v1", "faces", "soft", "curve", "layer",
                 "hidden", "flat")

    def __init__(self, v0: Vertex, v1: Vertex, *, key: int | None = None) -> None:
        self.key = next(_STANDALONE_KEYS) if key is None else key
        self.v0 = v0
        self.v1 = v1
        self.faces: list[Face] = []
        # A "soft" edge is a curve segment (circle/arc): kept in the topology but
        # hidden from the edge render, so the curve reads smooth.
        self.soft: bool = False
        # A soft edge shades smooth across it — the modeller's soft edge with
        # «Smooth normals» on, and what every soft edge was before the two
        # were told apart (a file without this flag reads that way). A
        # ``flat`` soft edge is soft only: its line leaves the render but
        # the faces on either side keep their own flat shading (Soften
        # Edges with «Smooth normals» off). Meaningless on a hard edge.
        self.flat: bool = False
        # A "hidden" edge is fully invisible: unlike soft it draws neither
        # as a line nor as a profile/silhouette (imported foliage cards —
        # the leaf's outline is its texture mask, not its quad).
        self.hidden: bool = False
        # Layer / tag name (None = default layer).
        self.layer = None
        # Segments of one drawn curve (circle/arc) share a ``curve`` id, so
        # selecting one segment selects the whole curve (a curve entity).
        # ``None`` for a plain edge. A split just leaves each side's segments with
        # the same id → the pieces select as separate arcs.
        self.curve: int | None = None

    # Position accessors keep parity with the legacy ``Edge.a`` / ``Edge.b`` so
    # read-only consumers (render, bounds) port with minimal churn.
    @property
    def a(self) -> QVector3D:
        return self.v0.position

    @property
    def b(self) -> QVector3D:
        return self.v1.position

    def other(self, v: Vertex) -> Vertex:
        """The endpoint that is not ``v``."""
        return self.v1 if v == self.v0 else self.v0

    def has(self, v: Vertex) -> bool:
        return v == self.v0 or v == self.v1

    def length(self) -> float:
        return (self.v1.position - self.v0.position).length()

    def __repr__(self) -> str:  # pragma: no cover - debug aid
        return f"Edge({self.v0!r} -> {self.v1!r})"


#: The ``Face.attrs`` keys that make up a face's PAINT — how it looks (colour
#: or texture, and its opacity) plus the material identity that survives the
#: churn. ``layer`` and ``ifc`` are deliberately NOT here: they travel by their
#: own rules (new geometry belongs to the active tag, and a BIM tag extends its
#: object). Anything that hands paint from one face to another — a face carved
#: out of another, an extrusion's walls — keys on these.
PAINT_KEYS = ("color", "mat", "texture", "opacity", "pbr")

#: Everything an edge carries besides its two ends, in the order
#: :meth:`Mesh.add_edges_welded` reads them.
#:
#: Any code that rebuilds a mesh from bare coordinates has to carry these
#: across, and forgetting one is invisible until a user hits it. Grouping a
#: cylinder once showed every facet seam because ``soft`` was dropped; then
#: Marco hid an edge, grouped the drawing, and the edge came back (2026-09-10)
#: because ``hidden`` was added to ``Edge`` later and only some of the copies
#: learned about it. Adding a flag here now teaches every one of them at once.
EDGE_FLAG_NAMES = ("soft", "curve", "layer", "hidden", "flat")
EDGE_FLAG_DEFAULTS = (False, None, None, False, False)


def smooths(edge) -> bool:
    """Whether the shading runs smooth across ``edge``: a soft edge not
    marked ``flat`` (Soften Edges' «Smooth normals»)."""
    return bool(getattr(edge, "soft", False)) and not getattr(edge, "flat", False)


def edge_flags(edge) -> tuple:
    """The edge's flags as a tuple, ready to travel with its coordinates."""
    return tuple(getattr(edge, name, default)
                 for name, default in zip(EDGE_FLAG_NAMES, EDGE_FLAG_DEFAULTS))


def edge_is_plain(edge) -> bool:
    """True when there is nothing to carry: a bare, visible, untagged edge."""
    return edge_flags(edge) == EDGE_FLAG_DEFAULTS


def stamp_edge_flags(edge, flags) -> None:
    """Put back what :func:`edge_flags` took."""
    for name, value in zip(EDGE_FLAG_NAMES, flags):
        setattr(edge, name, value)


class Face:
    """A planar polygon: an outer ``loop`` of shared vertices, plus optional
    inner ``hole_loops``. Geometry is derived from the vertices' positions, so
    it follows automatically when a shared vertex moves.

    Connectivity is kept as vertex loops (``loop`` / ``hole_loops``). The
    ``vertices`` and ``holes`` properties return the same loops as plain
    positions, matching the legacy ``core.geometry.Face`` read interface — so
    rendering, bounds and ``.igz`` save consume a ``mesh.Face`` unchanged (M1).
    """

    __slots__ = ("key", "loop", "hole_loops", "interior", "attrs")

    def __init__(
        self, loop: list[Vertex], hole_loops: Optional[list[list[Vertex]]] = None,
        *, key: int | None = None
    ) -> None:
        self.key = next(_STANDALONE_KEYS) if key is None else key
        self.loop = list(loop)
        self.hole_loops = [list(h) for h in hole_loops] if hole_loops else []
        # Interior partition: a face *inside* a solid (the slab a Ctrl-push
        # keeps, a wall two rooms share) rather than on its boundary. Marked by
        # ``core.orient.orient_outward`` (and the plane rebuild) so volumetric
        # queries — parity ray-casting, signed volume — skip it: an interior
        # face is not part of the boundary, and counting a ray crossing
        # through it would corrupt every inside/outside answer.
        self.interior = False
        # User-facing attributes that must survive the engine's face churn
        # (materials, future BIM tags). The rebuild/dissolve/dedupe/fold paths
        # replace face objects, so each inheritance point carries this dict to
        # the surviving/covering face — see tests/test_face_attrs.py.
        self.attrs: dict = {}

    def reverse_rings(self) -> None:
        """Reverse the winding of every ring, preserving paint and incidence."""
        self.loop.reverse()
        for hole in self.hole_loops:
            hole.reverse()

    # ---- Legacy-compatible read interface (positions) -----------------------
    @property
    def vertices(self) -> list[QVector3D]:
        return [v.position for v in self.loop]

    @property
    def holes(self) -> list[list[QVector3D]]:
        return [[v.position for v in h] for h in self.hole_loops]

    # ---- Geometry (ported from legacy Face, over shared positions) ----------
    def _newell(self) -> QVector3D:
        """Newell's area-weighted normal, accumulated in DOUBLE precision.

        Summing in ``QVector3D`` (float32) left a horizontal face — every
        vertex at the same z — with a normal like ``(2.9e-6, 0, 1)``, and
        small faces far from the origin up to ``(6e-4, 0, 1)``. Harmless for
        shading, fatal for texture projection: the .skp basis is ``Z × n``,
        which snaps to the world axes only for a vertical normal, so that
        noise turned the projection 90° against the plane the .skp writer
        computes in float64 from the same vertices (Marco's pool: every
        horizontal countertop, floor and slab of the .skp). Doubles make
        a constant-z face exactly vertical, like the writer's own plane."""
        loop = self.loop
        count = len(loop)
        nx = ny = nz = 0.0
        for i in range(count):
            curr = loop[i].position
            nxt = loop[(i + 1) % count].position
            x0, y0, z0 = curr.x(), curr.y(), curr.z()
            x1, y1, z1 = nxt.x(), nxt.y(), nxt.z()
            nx += (y0 - y1) * (z0 + z1)
            ny += (z0 - z1) * (x0 + x1)
            nz += (x0 - x1) * (y0 + y1)
        return QVector3D(nx, ny, nz)

    def normal(self) -> QVector3D:
        if len(self.loop) < 3:
            return QVector3D(0.0, 0.0, 1.0)
        n = self._newell()
        length = n.length()
        if length < 1e-9:
            return QVector3D(0.0, 0.0, 1.0)
        # Divided by hand, NOT QVector3D.normalized(): Qt returns a NULL
        # vector for anything shorter than 1e-5 (its float qFuzzyIsNull) —
        # four orders of magnitude above the guard right above this line. So
        # a sliver whose Newell normal fell in [1e-9, 1e-5] got (0, 0, 0)
        # instead of the (0, 0, 1) this function promises, and a zero normal
        # is a WILDCARD: ``dot(anything, zero) == 0`` passes every plane
        # test. One 3 mm² sliver imported from a .skp file made every line
        # drawn anywhere in its group rebuild "its" plane, wiping all 447
        # faces (Plaza Yanque, 2026-09-10). Dividing keeps the real
        # direction, which is also strictly more informative.
        return n / length

    def area(self) -> float:
        if len(self.loop) < 3:
            return 0.0
        return 0.5 * self._newell().length()

    def net_area(self) -> float:
        """``area()`` with the hole loops subtracted: the surface a
        person measuring the face means — a wall's window opening is
        not wall. ``area()`` itself stays gross on purpose (the engine
        reads it: heal thresholds, the dominant normal), so the
        user-facing numbers — Entity Info, quantities, labels — read
        this one instead (#742)."""
        if len(self.loop) < 3:
            return 0.0
        area = self.area()
        for hole in self.hole_loops:
            count = len(hole)
            if count < 3:
                continue
            nx = ny = nz = 0.0
            for i in range(count):
                curr = hole[i].position
                nxt = hole[(i + 1) % count].position
                x0, y0, z0 = curr.x(), curr.y(), curr.z()
                x1, y1, z1 = nxt.x(), nxt.y(), nxt.z()
                nx += (y0 - y1) * (z0 + z1)
                ny += (z0 - z1) * (x0 + x1)
                nz += (x0 - x1) * (y0 + y1)
            area -= 0.5 * QVector3D(nx, ny, nz).length()
        return max(area, 0.0)

    def centroid(self) -> QVector3D:
        count = len(self.loop)
        if count == 0:
            return QVector3D(0.0, 0.0, 0.0)
        cx = sum(v.position.x() for v in self.loop) / count
        cy = sum(v.position.y() for v in self.loop) / count
        cz = sum(v.position.z() for v in self.loop) / count
        return QVector3D(cx, cy, cz)

    def triangulate(self, normal: QVector3D | None = None
                    ) -> list[tuple[QVector3D, QVector3D, QVector3D]]:
        if len(self.loop) < 3:
            return []
        from core.triangulate import triangulate

        return triangulate(self.vertices, self.holes,
                           normal if normal is not None else self.normal())

    def __repr__(self) -> str:  # pragma: no cover - debug aid
        return f"Face({len(self.loop)} verts, {len(self.holes)} holes)"


class Mesh:
    """Owns the shared vertices, edges and faces and keeps their incidence in
    sync. The single place geometry is mutated; all welding happens here.
    """

    def __init__(self) -> None:
        self._keys = count(next(_KEY_NAMESPACES) << 40)
        if type(self) is Mesh and _store_on():
            from core.store_mesh import initialise_store
            self._mut_serial = 0
            self._chunk_dirty = True
            initialise_store(self)
            return
        self._init_object_geometry()

    def _init_object_geometry(self):
        self._registry: dict[tuple, Vertex] = {}
        # Moved vertices waiting for a key another vertex still holds (see
        # _rekey).
        self._displaced: dict[tuple, list[Vertex]] = {}
        #: Set by every mutation primitive; the viewport's cached group chunk
        #: clears it after (re)validating — an O(1) dirty signal so render
        #: caches never content-hash a 131k-vertex mesh per scene change.
        self._chunk_dirty = True
        # Bumped by every mutation primitive: a cheap 'has this mesh
        # changed since I looked' for caches that must never build a
        # chunk to find out (the container box's point arrays).
        self._mut_serial = 0
        self.vertices: list[Vertex] = []
        self.edges: list[Edge] = []
        self.faces: list[Face] = []

    def __deepcopy__(self, memo):
        # A copied mesh is a new owner, not an undo snapshot. Give its elements
        # a separate namespace so scene-wide caches cannot confuse the copies.
        from copy import deepcopy
        if isinstance(self, DeferredMesh):
            self._materialize()
            if type(self) is not Mesh:
                return self.__deepcopy__(memo)     # store-backed now
        copied = object.__new__(Mesh)
        copied._keys = count(next(_KEY_NAMESPACES) << 40)
        memo[id(self)] = copied
        copied.__dict__.update(deepcopy(
            {k: v for k, v in self.__dict__.items()
             if k != "_keys" and k not in _STORE_SIDE}, memo))
        for element in (*copied.vertices, *copied.edges, *copied.faces):
            element.key = next(copied._keys)
        return copied

    @property
    def revision(self) -> int:
        """Session-local change token; reading never builds a deferred mesh."""
        return self.__dict__.get("_revision", 0) + self._mut_serial

    @property
    def _chunk_dirty(self):
        return self.__dict__.get("_chunk_dirty_value", True)

    @_chunk_dirty.setter
    def _chunk_dirty(self, value):
        self.__dict__["_chunk_dirty_value"] = value
        if value:
            self.bump_revision()

    @property
    def _attrs_dirty(self):
        return self.__dict__.get("_attrs_dirty_value", False)

    @_attrs_dirty.setter
    def _attrs_dirty(self, value):
        self.__dict__["_attrs_dirty_value"] = value
        if value:
            self.bump_revision()

    def invalidate_placement_chunks(self) -> None:
        """Refresh chunks that bake placement state. The mesh itself is unchanged."""
        self.__dict__["_chunk_dirty_value"] = True
        self.__dict__["_attrs_dirty_value"] = True

    def bump_revision(self) -> None:
        """Advance :attr:`revision` alone: a change readers of the revision
        must see that touched no coordinate — an attribute, a tag, a
        material. The mutation serial and the winding count stay put, so
        the viewport's geometry caches (keyed on those) are not thrown
        away for a paint; see :meth:`mark_modified` for raw edits."""
        self.__dict__["_revision"] = self.__dict__.get("_revision", 0) + 1
        definition = self.__dict__.get("_instance_definition")
        if definition is not None:
            definition.mark_modified()

    def mark_modified(self) -> None:
        """Publish a batch of direct element edits, including attributes."""
        self._chunk_dirty = True
        self._attrs_dirty = True
        # Raw position and winding edits must invalidate readers that key
        # on geometry alone, not just the renderer's material dirty flag.
        self._mut_serial += 1
        self._winding_rev = getattr(self, "_winding_rev", 0) + 1

    # ---- Vertices -----------------------------------------------------------
    def _lookup(self, position: QVector3D) -> Optional[Vertex]:
        """The registered vertex within weld tolerance of ``position``, or
        ``None``. Rounding alone has a boundary hole: two points 2e-6 apart
        can straddle a 0.5e-4 rounding edge and land in different cells (a
        slanted plane's corner computed via two float paths) — so a miss in
        the exact cell probes the neighbouring cells and checks real
        distance."""
        k = _key(position)
        v = self._registry.get(k)
        if v is not None:
            return v
        return self._probe_neighbours(k, position)

    def _probe_neighbours(self, k, position: QVector3D) -> Optional[Vertex]:
        """The boundary-hole rescue of :meth:`_lookup`: probe the 26 cells
        around ``k`` and accept a vertex within real weld distance."""
        step = 10.0 ** -_KEY_DECIMALS
        tol2 = step * step
        kx, ky, kz = k
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                for dz in (-1, 0, 1):
                    if dx == 0 and dy == 0 and dz == 0:
                        continue
                    n = self._registry.get((kx + dx, ky + dy, kz + dz))
                    if n is not None and (
                            (n.position - position).lengthSquared() < tol2):
                        return n
        return None

    def vertex(self, position: QVector3D) -> Vertex:
        """Get-or-create the shared vertex at ``position`` (welds coincident
        points to one object)."""
        k = _key(position)
        v = self._registry.get(k)
        if v is None:
            v = self._probe_neighbours(k, position)
        if v is None:
            v = Vertex(position, key=next(self._keys))
            self._registry[k] = v
            self.vertices.append(v)
        return v

    def vertex_at(self, position: QVector3D) -> Optional[Vertex]:
        """The existing vertex at ``position``, or ``None``."""
        return self._lookup(position)

    def registers(self, v: Vertex) -> bool:
        """Whether this mesh files ``v`` itself — by identity, not by what
        stands at its position. A moved vertex that lands on another one
        for a moment (a Move preview passing over a corner) waits in
        ``_displaced`` while the other keeps the key: it is still this
        mesh's vertex, and a dimension holding it must not let go (#563).
        A vertex welded away or erased is filed nowhere."""
        k = _key(v.position)
        holder = self._registry.get(k)
        if holder is v:
            return True
        waiting = self._displaced.get(k)
        if waiting and any(w is v for w in waiting):
            return True
        if holder is None:
            return self._probe_neighbours(k, v.position) is v
        return False

    # ---- Edges --------------------------------------------------------------
    def find_edge(self, v0: Vertex, v1: Vertex) -> Optional[Edge]:
        """The edge between two vertices (either orientation), or ``None``.
        O(degree) via the smaller vertex's incidence — no list scan."""
        small = v0 if len(v0.edges) <= len(v1.edges) else v1
        target = v1 if small is v0 else v0
        for e in small.edges:
            if e.other(small) is target:
                return e
        return None

    @staticmethod
    def next_curve_id() -> int:
        """A fresh curve id — for tools that re-create curve edges (paste,
        offset) and must give the copy its own contour identity."""
        global _CURVE_COUNTER
        cid = _CURVE_COUNTER
        _CURVE_COUNTER += 1
        return cid

    def tag_curve(self, loop_points, closed: bool = True) -> int | None:
        """Mark every edge lying along the drawn ``loop_points`` path as one
        curve (a fresh id), so selecting any segment selects the whole circle/arc.
        Robust to splits: an edge is tagged when both its endpoints fall on one
        of the loop's segments, so a segment split by crossing geometry still has
        all its pieces tagged. Returns the id, or ``None`` if nothing matched."""
        self._chunk_dirty = True
        self._mut_serial += 1
        from core.topology import _point_on_seg_incl
        global _CURVE_COUNTER
        pts = list(loop_points)
        if len(pts) < 2:
            return None
        cid = _CURVE_COUNTER
        pairs = list(zip(pts, pts[1:]))
        if closed:
            pairs.append((pts[-1], pts[0]))
        tagged = False
        for e in self.edges:
            if e.curve is not None:
                continue
            a, b = e.a, e.b
            for pa, pb in pairs:
                if _point_on_seg_incl(a, pa, pb) and _point_on_seg_incl(b, pa, pb):
                    e.curve = cid
                    tagged = True
                    break
        if tagged:
            _CURVE_COUNTER += 1
            return cid
        return None

    def curve_edges(self, edge) -> list:
        """All edges sharing ``edge``'s curve id (``[edge]`` if it has none)."""
        cid = getattr(edge, "curve", None)
        if cid is None:
            return [edge]
        return [e for e in self.edges if e.curve == cid]

    def resplit_curves(self) -> None:
        """Re-partition curve ids into contiguous contours.

        A curve breaks where its own connectivity isn't a simple chain
        (curve-degree ≠ 2), or where other geometry attaches IN ITS PLANE (a
        line crossing or touching the circle). Each resulting chain
        keeps/gets its own id — so a circle crossed by a square's edges
        selects as two separate arcs ('two contours'). A pristine circle
        stays one curve, and so does the rim of a cylinder: its side seams
        leave the circle's plane, and counting them broke the rim into one
        curve per segment on every copy, explode or solid operation."""
        self._chunk_dirty = True
        self._mut_serial += 1
        by_id: dict[int, list[Edge]] = {}
        for e in self.edges:
            if e.curve is not None:
                by_id.setdefault(e.curve, []).append(e)
        self._resplit_curve_groups(by_id)

    @staticmethod
    def _resplit_curve_groups(by_id) -> None:
        """:meth:`resplit_curves` past its scan: ``by_id`` maps each curve
        id to its edges in mesh order."""
        global _CURVE_COUNTER
        for cid, edges in by_id.items():
            eset = set(edges)

            def is_break(v) -> bool:
                mine = [e for e in v.edges if e in eset]
                if len(mine) != 2:
                    return True
                if len(v.edges) == 2:
                    return False
                p = v.position
                a = mine[0].other(v).position - p
                b = mine[1].other(v).position - p
                n = QVector3D.crossProduct(a, b)
                if n.length() <= 1e-6 * a.length() * b.length():
                    return True              # straight through: no plane
                n.normalize()
                for e in v.edges:
                    if e in eset:
                        continue
                    u = e.other(v).position - p
                    length = u.length()
                    if (length <= 1e-12
                            or abs(QVector3D.dotProduct(u, n)) < 0.05 * length):
                        return True
                return False

            # Walk chains between break vertices; a break-free component is a
            # closed loop (one chain).
            unvisited = set(edges)
            chains: list[list[Edge]] = []
            # Seed from break vertices first so open chains are walked end-to-end.
            seeds = [e for e in edges if is_break(e.v0) or is_break(e.v1)]
            for seed in seeds + edges:
                if seed not in unvisited:
                    continue
                start_v = seed.v0 if is_break(seed.v0) else (
                    seed.v1 if is_break(seed.v1) else seed.v0)
                chain, v, e = [], start_v, seed
                while e in unvisited:
                    unvisited.discard(e)
                    chain.append(e)
                    v = e.other(v)
                    if is_break(v):
                        break
                    nxt = next((n for n in v.edges if n in unvisited), None)
                    if nxt is None:
                        break
                    e = nxt
                chains.append(chain)
            if len(chains) <= 1:
                continue  # still one contour — keep the id stable
            for i, chain in enumerate(chains):
                new_id = cid if i == 0 else _CURVE_COUNTER
                if i > 0:
                    _CURVE_COUNTER += 1
                for e in chain:
                    e.curve = new_id

    def _link_edge(self, v0: Vertex, v1: Vertex) -> Edge:
        self._chunk_dirty = True
        self._mut_serial += 1
        e = self.find_edge(v0, v1)
        if e is not None:
            return e
        e = Edge(v0, v1, key=next(self._keys))
        v0.edges.add(e)
        v1.edges.add(e)
        self.edges.append(e)
        return e

    def add_edge(self, a: QVector3D, b: QVector3D) -> Edge:
        """Add a free edge between two positions (welding endpoints, deduping
        the edge). Returns the existing edge if it is already there."""
        v0 = self.vertex(a)
        v1 = self.vertex(b)
        if v0 is v1:
            raise ValueError("degenerate edge: endpoints weld to one vertex")
        return self._link_edge(v0, v1)

    def remove_edge(self, e: Edge) -> None:
        """Detach and drop an edge. Callers remove any incident faces first when
        needed; this does not cascade."""
        self._chunk_dirty = True
        self._mut_serial += 1
        e.v0.edges.discard(e)
        e.v1.edges.discard(e)
        if e in self.edges:
            self.edges.remove(e)

    def prune_orphan_vertices(self) -> int:
        """Drop vertices nothing references any more. Returns how many went.

        ``remove_edge`` detaches an edge from its endpoints and leaves the
        endpoints in the mesh, so every erase used to leave its vertices
        behind. They do not draw and ``bounds()`` ignores them, which is why
        it went unseen for so long — Marco only hit it when a stray line 16 km
        out was deleted and the leftovers were all the document had, so
        zoom-to-extents had nothing to find and could not bring him back.

        The full sweep — this one plus the registry entries that point at
        vertices the mesh no longer owns — is :meth:`purge_debris`.
        """
        return self.purge_debris()[0]

    def purge_debris(self) -> tuple[int, int]:
        """Deep clean: drop the vertices nothing holds and the position
        entries that would hand one back. Returns
        ``(vertices dropped, ghost entries dropped)``.

        ``remove_face`` and ``remove_edge`` are deliberately additive — an
        edge may border another face, a corner may be left standing as a
        line — so whoever removes geometry that nothing else wants has to
        say so. Erasing does (``prune_orphan_vertices``); grouping did not:
        it carves faces and edges out of the loose mesh into a new container
        and the corners only they used stayed behind, invisible and
        unreferenced — eight of them for a whole box turned into a
        component. Nothing draws them, ``bounds()`` ignores them and the
        file format only stores what a face or an edge holds, so they die on
        save; but for the rest of the session they stay in the position
        registry, and the next thing drawn there welds to a ``Vertex`` the
        mesh no longer lists.

        That is the "ghost point" of the report: nothing draws it, nothing
        picks it, and yet a new face drawn at the spot ends up cut through
        it. The registry is therefore swept by IDENTITY, never by position
        key: an entry whose vertex is not in ``self.vertices`` goes, and so
        does a moved vertex still waiting in ``_displaced``.

        A face holds real ``Vertex`` objects in its loops, so "has no edges"
        is not enough on its own: the corners of a face with no free edges
        would go with it. Both are checked.
        """
        used = set()
        for f in self.faces:
            for v in f.loop:
                used.add(v.key)
            for hole in f.hole_loops:
                for v in hole:
                    used.add(v.key)
        doomed = {v.key for v in self.vertices
                  if not v.edges and v.key not in used}
        live = {v.key for v in self.vertices if v.key not in doomed}
        ghosts = [k for k, v in self._registry.items() if v.key not in live]
        for k in list(self._displaced):
            waiting = [v for v in self._displaced[k] if v.key in live]
            if waiting:
                self._displaced[k] = waiting
            else:
                del self._displaced[k]
        if not doomed and not ghosts:
            return 0, 0
        if doomed:
            self.vertices[:] = [v for v in self.vertices
                                if v.key not in doomed]
        for k in ghosts:
            del self._registry[k]
        self._chunk_dirty = True
        self._mut_serial += 1
        return len(doomed), len(ghosts)

    def _relist_corners(self, vertices) -> None:
        """Before a relink: list again each of ``vertices`` the mesh no
        longer has (a restore to a state older than the vertex, then the
        redo of the edge or face it carries), in order, so the relinked
        element never rests on an unlisted vertex."""
        listed = {v.key for v in self.vertices}
        for v in vertices:
            if v.key not in listed:
                self.vertices.append(v)
                listed.add(v.key)
                self._registry.setdefault(_key(v.position), v)

    def relink_vertices(self, vertices) -> None:
        """Re-attach vertices a previous :meth:`drop_orphan_vertices` took
        out (redo), preserving their identity; a position already filed
        under another vertex keeps that one."""
        present = {v.key for v in self.vertices}
        for v in vertices:
            if v.key in present:
                continue
            self.vertices.append(v)
            present.add(v.key)
            self._registry.setdefault(_key(v.position), v)
        self._chunk_dirty = True
        self._mut_serial += 1

    def drop_orphan_vertices(self, vertices) -> int:
        """Drop the given vertices that nothing references any more — the
        targeted twin of :meth:`prune_orphan_vertices`, for an undo that
        knows exactly which vertices it created (no pass over the model).
        A face's corners always carry its boundary edges, so a vertex with
        no edges is in no face. The registry is cleaned by identity."""
        doomed = {v.key for v in vertices if not v.edges}
        doomed &= {v.key for v in self.vertices}
        if not doomed:
            return 0
        self.vertices[:] = [v for v in self.vertices if v.key not in doomed]
        for v in vertices:
            if v.key in doomed:
                k = _key(v.position)
                if self._registry.get(k) is v:
                    del self._registry[k]
        self._chunk_dirty = True
        self._mut_serial += 1
        return len(doomed)

    # ---- Bulk construction (imports / .igz load) ----------------------------
    def bulk_weld(self, pos):
        """Weld a ``(P, 3)`` float array of positions into shared vertices —
        the exact vectorized equivalent of calling :meth:`vertex` on each
        point in order. Returns ``(vobjs, point_ids)``: the list of distinct
        :class:`Vertex` objects touched and a ``(P,)`` index array mapping
        each input point to its vertex in that list.

        Parity strategy: a cell with NO neighbouring cell present (in the
        batch or the pre-existing registry) can never weld across a boundary
        — all its points resolve to one vertex at its first point, no probe
        needed. Points in *candidate* cells (a neighbour exists) replay the
        sequential per-point logic exactly, probes and all — including the
        subtle case where a probe welds a point WITHOUT claiming its cell,
        so a later, farther point of the same cell creates its own vertex."""
        import numpy as np
        d = self.__dict__
        if (type(self) is Mesh and not d["vertices"] and not d["edges"]
                and not d["faces"] and not d["_registry"] and _store_on()):
            # An empty mesh built in bulk (an import, a group, a copy):
            # into the Rust store (core/store_mesh.py), same vertex order.
            from core import store_mesh
            return store_mesh.bulk_weld(self, pos)
        from core.store_mesh import StoreMesh
        store = self._store if isinstance(self, StoreMesh) else None
        pos = np.ascontiguousarray(pos, dtype=np.float64)
        if not len(pos):
            return [], np.empty(0, dtype=np.int64)
        model = store._definition_points(pos) if store is not None and not store._model_frame else pos
        keys = np.round(model * _KEY_SCALE).astype(np.int64)
        # Byte records group equal int64 triples without the field-by-field
        # structured sort. Only equality matters: first occurrence restores
        # the cell order below, regardless of the byte sort's ordering.
        view = np.ascontiguousarray(keys).view("V24").reshape(-1)
        uview, first, inverse = np.unique(
            view, return_index=True, return_inverse=True)
        order = np.argsort(first, kind="stable")
        rank = np.empty_like(order)
        rank[order] = np.arange(len(order))
        inverse = rank[inverse]                # point → unique cell (creation order)
        first = first[order]
        ukeys = keys[first]                    # unique cells, creation order
        nu = len(first)
        candidate = _weld_candidates(ukeys)
        registry = (set(map(tuple, store.registry()[0].tolist())) if store is not None
                    else self._registry)
        if registry:
            # Existing cells participate in the neighbourhood test too.
            key_rows = ukeys.tolist()
            flat = ~candidate
            for u in np.flatnonzero(flat).tolist():
                kx, ky, kz = key_rows[u]
                found = False
                for dx in (-1, 0, 1):
                    for dy in (-1, 0, 1):
                        for dz in (-1, 0, 1):
                            if (kx + dx, ky + dy, kz + dz) in registry:
                                found = True
                                break
                        if found:
                            break
                    if found:
                        break
                candidate[u] = found
        vertices = self.vertices
        vobjs: list = []
        vindex: dict = {}

        def _vid(v):
            i = vindex.get(v.key)
            if i is None:
                i = len(vobjs)
                vindex[v.key] = i
                vobjs.append(v)
            return i

        point_ids = np.empty(len(pos), dtype=np.int64)
        cand_pt = candidate[inverse]
        # Non-candidate cells: one vertex at the cell's first point.
        nc_cells = np.flatnonzero(~candidate)
        cell_vid = np.full(nu, -1, dtype=np.int64)
        if len(nc_cells):
            nc_keys = ukeys[nc_cells].tolist()
            nc_pos = pos[first[nc_cells]].tolist()
            ids = np.empty(len(nc_cells), dtype=np.int64)
            for j, (k3, p3) in enumerate(zip(nc_keys, nc_pos)):
                if store is None:
                    v = Vertex(QVector3D(*p3), key=next(self._keys))
                    registry[tuple(k3)] = v
                    vertices.append(v)
                else:
                    v = self.vertex(p3)
                ids[j] = _vid(v)
            cell_vid[nc_cells] = ids
            ncp = ~cand_pt
            point_ids[ncp] = cell_vid[inverse[ncp]]
        # Candidate points: exact sequential replay, in input order.
        cp = np.flatnonzero(cand_pt)
        if len(cp):
            cp_list = cp.tolist()
            cp_keys = keys[cp].tolist()
            cp_pos = pos[cp].tolist()
            probe = self._probe_neighbours
            for j, i in enumerate(cp_list):
                if store is not None:
                    point_ids[i] = _vid(self.vertex(cp_pos[j]))
                    continue
                k3 = cp_keys[j]
                k = (k3[0], k3[1], k3[2])
                v = registry.get(k)
                if v is None:
                    p3 = cp_pos[j]
                    p = QVector3D(p3[0], p3[1], p3[2])
                    v = probe(k, p)
                    if v is None:
                        v = Vertex(p, key=next(self._keys))
                        registry[k] = v
                        vertices.append(v)
                point_ids[i] = _vid(v)
        return vobjs, point_ids

    def add_edges_welded(self, vobjs, ia, ib, flags=None):
        """Create the edges ``(vobjs[ia[i]], vobjs[ib[i]])`` — the vectorized
        equivalent of :meth:`add_edge` per pair: endpoints that welded to one
        vertex are skipped (the ``ValueError`` case), duplicates return the
        existing edge. ``flags`` is an optional per-edge ``(soft, curve,
        layer[, hidden[, flat]])`` tuple list. Returns ``{packed pair: Edge}`` over ``vobjs``
        indices — pass it to :meth:`add_faces_welded` so face boundaries
        reuse these edges without rescanning ``self.edges``."""
        self._chunk_dirty = True
        self._mut_serial += 1
        u = len(vobjs)
        emap: dict = {}
        ia = list(ia.tolist() if hasattr(ia, "tolist") else ia)
        ib = list(ib.tolist() if hasattr(ib, "tolist") else ib)
        for i in range(len(ia)):
            a = ia[i]
            b = ib[i]
            v0 = vobjs[a]
            v1 = vobjs[b]
            if v0 is v1:
                continue                       # degenerate: endpoints welded
            key = a * u + b if a <= b else b * u + a
            e = emap.get(key)
            if e is None:
                e = Edge(v0, v1, key=next(self._keys))
                v0.edges.add(e)
                v1.edges.add(e)
                self.edges.append(e)
                emap[key] = e
            if flags is not None:
                soft, curve, layer = flags[i][:3]
                if soft:
                    e.soft = True
                if len(flags[i]) > 3 and flags[i][3]:
                    e.hidden = True
                if len(flags[i]) > 4 and flags[i][4]:
                    e.flat = True
                if curve is not None:
                    e.curve = curve
                if layer is not None:
                    e.layer = layer
        return emap

    def add_faces_welded(self, vobjs, inverse, ring_sizes, face_ring_counts,
                         attrs=None, edge_map=None):
        """Create faces from pre-welded corners — the vectorized equivalent
        of :meth:`add_face` per face, minus the nested-hole filter (callers
        run :func:`core.topology._maximal_holes` while flattening, where the
        loops still exist as positions).

        ``inverse`` indexes every ring corner into ``vobjs``, rings
        concatenated in face order (outer first, then that face's holes);
        ``ring_sizes`` is each ring's corner count and ``face_ring_counts``
        how many rings each face owns. ``attrs`` (optional) is one dict or
        ``None`` per face, applied via ``face.attrs.update``. ``edge_map``
        is the pair→edge dict from :meth:`add_edges_welded` (or ``None`` to
        derive it from ``self.edges``). Returns the new faces in order."""
        import numpy as np
        self._chunk_dirty = True
        self._mut_serial += 1
        u = len(vobjs)
        inverse = np.ascontiguousarray(inverse, dtype=np.int64)
        ring_sizes = np.ascontiguousarray(ring_sizes, dtype=np.int64)
        face_ring_counts = np.ascontiguousarray(face_ring_counts,
                                                dtype=np.int64)
        if not len(face_ring_counts):
            return []
        ring_ends = np.cumsum(ring_sizes)
        ring_starts = ring_ends - ring_sizes
        total = int(ring_ends[-1]) if len(ring_ends) else 0
        # Boundary segments: each corner to the next within its ring.
        nxt = np.arange(1, total + 1, dtype=np.int64)
        nxt[ring_ends - 1] = ring_starts
        seg_a = inverse
        seg_b = inverse[nxt]
        packed = np.minimum(seg_a, seg_b) * u + np.maximum(seg_a, seg_b)
        if edge_map is None:
            edge_map = {}
            if self.edges:
                vid_of = {v.key: i for i, v in enumerate(vobjs)}
                for e in self.edges:
                    i0 = vid_of.get(e.v0.key)
                    i1 = vid_of.get(e.v1.key)
                    if i0 is not None and i1 is not None:
                        key = i0 * u + i1 if i0 <= i1 else i1 * u + i0
                        edge_map.setdefault(key, e)
        upacked, efirst, einv = np.unique(
            packed, return_index=True, return_inverse=True)
        eorder = np.argsort(efirst, kind="stable")
        erank = np.empty_like(eorder)
        erank[eorder] = np.arange(len(eorder))
        einv = erank[einv]
        upacked = upacked[eorder]
        efirst = efirst[eorder]
        eobjs: list = []
        seg_a_list = seg_a.tolist()
        seg_b_list = seg_b.tolist()
        for key, src in zip(upacked.tolist(), efirst.tolist()):
            e = edge_map.get(key)
            if e is None:
                v0 = vobjs[seg_a_list[src]]    # first occurrence's orientation
                v1 = vobjs[seg_b_list[src]]
                e = Edge(v0, v1, key=next(self._keys))
                v0.edges.add(e)
                v1.edges.add(e)
                self.edges.append(e)
                edge_map[key] = e
            eobjs.append(e)
        # Face objects, loops sliced from the welded corner list (plain list
        # slicing — np.split into hundreds of thousands of views is slow).
        corner_list = [vobjs[i] for i in inverse.tolist()]
        rs_list = ring_starts.tolist()
        re_list = ring_ends.tolist()
        faces_out = []
        ri = 0
        for nr in face_ring_counts.tolist():
            loop = corner_list[rs_list[ri]:re_list[ri]]
            holes = [corner_list[rs_list[ri + k]:re_list[ri + k]]
                     for k in range(1, nr)]
            ri += nr
            face = Face(loop, holes or None, key=next(self._keys))
            if attrs is not None:
                a = attrs[len(faces_out)]
                if a:
                    face.attrs.update(a)
            self.faces.append(face)
            faces_out.append(face)
        # Radial incidence: every boundary segment registers its face on its
        # edge, in global segment order (= add_face's per-face order),
        # multiplicity preserved.
        seg_face = np.repeat(np.arange(len(face_ring_counts), dtype=np.int64),
                             np.add.reduceat(ring_sizes,
                                             np.cumsum(face_ring_counts)
                                             - face_ring_counts))
        order2 = np.argsort(einv, kind="stable")
        sorted_faces = [faces_out[i] for i in seg_face[order2].tolist()]
        bounds = np.cumsum(np.bincount(einv, minlength=len(eobjs))).tolist()
        b0 = 0
        for ei, b1 in enumerate(bounds):
            if b1 > b0:
                eobjs[ei].faces.extend(sorted_faces[b0:b1])
            b0 = b1
        return faces_out

    def add_faces_bulk(self, pos, ring_sizes, face_ring_counts, attrs=None):
        """Convenience: :meth:`bulk_weld` + :meth:`add_faces_welded`."""
        vobjs, inverse = self.bulk_weld(pos)
        return self.add_faces_welded(vobjs, inverse, ring_sizes,
                                     face_ring_counts, attrs)

    # ---- Faces --------------------------------------------------------------
    def add_face(
        self,
        loop_positions: Iterable[QVector3D],
        hole_loops: Optional[Iterable[Iterable[QVector3D]]] = None,
    ) -> Face:
        """Add a face from position loops. Vertices weld, the boundary edges are
        created if missing (and the face is registered on each as an incident
        face — radial, so an edge can carry several).

        Nested holes are dropped here, at the single choke point every face
        creation path goes through (draw, push rebuild, heal, ``.igz`` load): a
        hole inside another hole of the same face is geometric nonsense — its
        region is already void — yet it corrupts earcut (phantom wedge
        triangles across the opening) and registers the face on rim edges it
        does not geometrically touch, which lets ``is_closed`` report a broken
        solid as watertight (defeating the BIM push guard)."""
        self._chunk_dirty = True
        self._mut_serial += 1
        loop = [self.vertex(p) for p in loop_positions]
        raw_holes = [list(h) for h in (hole_loops or [])]
        if len(raw_holes) > 1:
            from core.topology import _maximal_holes

            raw_holes = _maximal_holes(raw_holes)
        holes = [[self.vertex(p) for p in h] for h in raw_holes]
        face = Face(loop, holes, key=next(self._keys))  # Face stores them as hole_loops (vertices)
        for lp in (loop, *holes):
            n = len(lp)
            for i in range(n):
                edge = self._link_edge(lp[i], lp[(i + 1) % n])
                edge.faces.append(face)
        self.faces.append(face)
        return face

    def surface_of(self, face: "Face") -> list:
        """The faces forming the same **curved surface** as ``face`` — the
        connected component reached by crossing **soft** edges (a circle/arc
        sweep's hidden seams). Faces joined by softened edges form one
        surface, so clicking/painting one acts on the whole curved
        side. Returns just ``[face]`` for ordinary (hard-edged) geometry."""
        seen = {face}
        stack = [face]
        while stack:
            f = stack.pop()
            for lp in (f.loop, *f.hole_loops):
                n = len(lp)
                for i in range(n):
                    e = self.find_edge(lp[i], lp[(i + 1) % n])
                    if e is None or not e.soft:
                        continue
                    for g in e.faces:
                        if g != f and g not in seen:
                            seen.add(g)
                            stack.append(g)
        return list(seen)

    def remove_face(self, face: Face) -> None:
        """Drop a face and detach it from its boundary edges' radial lists. The
        edges themselves stay (they may border other faces or stand alone)."""
        self._chunk_dirty = True
        self._mut_serial += 1
        for lp in (face.loop, *face.hole_loops):
            n = len(lp)
            for i in range(n):
                edge = self.find_edge(lp[i], lp[(i + 1) % n])
                if edge is not None and face in edge.faces:
                    edge.faces.remove(face)
        if face in self.faces:
            self.faces.remove(face)

    # ---- Re-link (undo support) --------------------------------------------
    def relink_edge(self, edge: Edge) -> None:
        """Re-attach a previously removed edge object (undo), preserving its
        identity so other commands' references stay valid."""
        if edge in self.edges:
            return
        self._relist_corners((edge.v0, edge.v1))
        edge.v0.edges.add(edge)
        edge.v1.edges.add(edge)
        self.edges.append(edge)
        self._registry.setdefault(_key(edge.v0.position), edge.v0)
        self._registry.setdefault(_key(edge.v1.position), edge.v1)
        self.mark_modified()

    def relink_face(self, face: Face) -> None:
        """Re-attach a previously removed face object (undo), recreating any
        missing boundary edges and re-registering incidence."""
        if face in self.faces:
            return
        self._relist_corners([v for lp in (face.loop, *face.hole_loops) for v in lp])
        for lp in (face.loop, *face.hole_loops):
            n = len(lp)
            for i in range(n):
                edge = self._link_edge(lp[i], lp[(i + 1) % n])
                if face not in edge.faces:
                    edge.faces.append(face)
        self.faces.append(face)
        self.mark_modified()

    # ---- Holes --------------------------------------------------------------
    def add_hole(self, face: Face, loop_positions: Iterable[QVector3D]) -> list[Vertex]:
        """Punch a hole into ``face`` (a window/door drawn inside it). Welds the
        loop's vertices, ensures its boundary edges, and registers the face on
        them. Returns the vertex loop so undo can remove exactly this hole."""
        loop = [self.vertex(p) for p in loop_positions]
        face.hole_loops.append(loop)
        n = len(loop)
        for i in range(n):
            edge = self._link_edge(loop[i], loop[(i + 1) % n])
            if face not in edge.faces:
                edge.faces.append(face)
        return loop

    def relink_hole(self, face: Face, ring: list[Vertex]) -> None:
        """Restore a detached hole and its incidence on existing edges."""
        face.hole_loops.append(ring)
        for i, vertex in enumerate(ring):
            edge = self.find_edge(vertex, ring[(i + 1) % len(ring)])
            if edge is not None and face not in edge.faces:
                edge.faces.append(face)

    def remove_hole(self, face: Face, loop: list[Vertex]) -> None:
        if loop in face.hole_loops:
            face.hole_loops.remove(loop)
        n = len(loop)
        for i in range(n):
            edge = self.find_edge(loop[i], loop[(i + 1) % n])
            if edge is not None and face in edge.faces:
                edge.faces.remove(face)

    # ---- State snapshot (identity-preserving) -------------------------------
    def capture_state(self) -> dict:
        """Snapshot the mesh keeping *object identity*: the same vertex/edge/face
        objects, plus a copy of every mutable field. :meth:`restore_state` brings
        the exact objects back, including their stable keys. The allocation
        counter is deliberately not restored: discarded edits never free keys.

        Used by the stitch pass, whose splits/merges/collapses interact too much
        for a clean per-op inverse — restoring the whole touched mesh is robust."""
        from ingetrazo_core import object_replay
        return object_replay.capture(self)

    def restore_state(self, snap: dict) -> None:
        """Restore a :meth:`capture_state` snapshot. Objects created since are
        dropped; captured ones are re-listed with their fields reset.
        A store snapshot (``core.store_mesh.StoreState``) restores this mesh
        to its store representation."""
        if not isinstance(snap, dict):
            from core import store_mesh
            store_mesh._restore_store_state(self, snap)
            return
        from ingetrazo_core import object_replay
        return object_replay.restore(self, snap)

    def note_rewound(self) -> None:
        """Say that faces may have changed winding (#197): no count,
        coordinate or paint shows it, so the drawing's chunk has to be told.
        Marked dirty too, or a move in the same breath would carry the old
        triangles along on the translation fast path."""
        self._winding_rev = getattr(self, "_winding_rev", 0) + 1
        self._chunk_dirty = True
        self.__dict__["_attrs_dirty_value"] = True   # one revision bump

    # ---- Reset --------------------------------------------------------------
    def clear(self) -> None:
        self._chunk_dirty = True
        self._mut_serial += 1
        self._registry.clear()
        self._displaced.clear()
        self.vertices.clear()
        self.edges.clear()
        self.faces.clear()

    # ---- Incidence queries --------------------------------------------------
    def edges_at(self, v: Vertex) -> set[Edge]:
        return set(v.edges)

    def faces_at(self, v: Vertex) -> set[Face]:
        return v.faces()

    # ---- Mutation -----------------------------------------------------------
    def move_vertex(self, v: Vertex, delta: QVector3D) -> None:
        """Translate a vertex. Every edge and face referencing it follows for
        free (they hold the same object) — the headline win over the legacy
        position-matching move.

        The registry is re-keyed so later welds find it at the new spot. Merging
        when it lands exactly on another vertex is deliberately *not* done here;
        that topological merge is a separate operation (migration phase M2).
        """
        self._chunk_dirty = True
        self._mut_serial += 1
        old = _key(v.position)
        v.position = v.position + delta
        new = _key(v.position)
        if old == new:
            return
        reg = self._registry
        if not self._displaced and reg.get(old) is v and new not in reg:
            del reg[old]               # the common case, kept inline
            reg[new] = v
            return
        self._rekey(v, old, new)

    def place_vertex(self, v: Vertex, position: QVector3D) -> None:
        """Put a vertex AT ``position`` -- :meth:`move_vertex` with an
        absolute target. ``p + (q - p)`` is not ``q`` in single precision,
        so a preview that restores saved positions through deltas left
        nanometres of drift behind (issue #163); this assigns them."""
        self._chunk_dirty = True
        self._mut_serial += 1
        old = _key(v.position)
        v.position = QVector3D(position)
        new = _key(v.position)
        if old == new:
            return
        reg = self._registry
        if not self._displaced and reg.get(old) is v and new not in reg:
            del reg[old]               # the common case, kept inline
            reg[new] = v
            return
        self._rekey(v, old, new)

    def _rekey(self, v: Vertex, old: tuple, new: tuple) -> None:
        """File a moved vertex under its new key.

        A selection moved along itself -- two chained lines shifted one
        length down their own axis -- moves each vertex onto the spot the
        next one still holds. The newcomer cannot take that key yet, so it
        waits in ``_displaced``; when the holder moves on, the first waiter
        still standing there takes the key. Without the queue the moved
        vertex was never filed again: Delete missed its edges and a new line
        to it welded a duplicate."""
        if self._registry.get(old) is v:
            del self._registry[old]
            for w in self._displaced.pop(old, ()):
                if w is not v and w.edges and _key(w.position) == old:
                    self._registry[old] = w
                    break
        else:
            waiting = self._displaced.get(old)
            if waiting is not None:
                waiting[:] = [w for w in waiting if w is not v]
                if not waiting:
                    del self._displaced[old]
        holder = self._registry.get(new)
        if holder is None:
            self._registry[new] = v
        elif holder is not v:
            self._displaced.setdefault(new, []).append(v)

    def split_edge(self, edge: Edge, position: QVector3D) -> tuple[Edge, Edge]:
        """Split ``edge`` at ``position``, inserting a shared vertex, and return
        the two sub-edges. A new first half (v0 to mid) keeps the original
        key; an existing first half keeps its own key. The second half keeps
        its existing key or gets a fresh one. Faces and endpoints keep keys.

        Every incident face gains the new vertex in its loop, between the edge's
        endpoints — for *any* number of faces (non-manifold). This is the
        operation that, in the legacy model, needed position-matching plus a
        special ``split_edge_in_faces`` pass plus the holes patch; here it falls
        straight out of shared connectivity. The new vertex lands collinearly,
        so faces stay flat until it is later moved (a gable ridge, a T-junction).

        A face may walk the edge more than once: a slit (a line from its
        outline to a hole, walked up and back) or a jamb two of its holes
        share. The vertex goes into every use, and each half lists the face
        once per use, as :meth:`add_face` lists it — the first use alone
        left the others a segment with no edge.
        """
        from ingetrazo_core import split_object_at_position
        return split_object_at_position(self, edge, position)

    @staticmethod
    def _insert_into_face_loops(
        face: Face, v0: Vertex, v1: Vertex, mid: Vertex
    ) -> None:
        """Insert ``mid`` between the consecutive ``v0``/``v1`` pair in the
        face's outer loop or one of its holes (whichever carries that edge)."""
        for loop in (face.loop, *face.hole_loops):
            n = len(loop)
            for i in range(n):
                j = (i + 1) % n
                if (loop[i] is v0 and loop[j] is v1) or (
                    loop[i] is v1 and loop[j] is v0
                ):
                    loop.insert(i + 1, mid)  # i+1 == n wraps to an append
                    return

    # ---- Stitch primitives (watertight cleanup, all reversible) -------------
    def weld_coincident(self) -> bool:
        """Merge vertices occupying the same position — the debris a translation
        leaves when geometry lands exactly on existing geometry (a cap pushed
        flush onto the ring it came from). Edges and face loops are re-pointed
        onto the kept vertex; zero-length edges vanish, duplicate edges merge
        their incidence, and faces degenerated below three distinct vertices
        are dropped. Returns whether anything merged. No undo bookkeeping —
        callers run under a snapshot, like the rest of the stitch.
        The first vertex in mesh order keeps its key within a cell; across
        cells the lexicographically smaller position cell wins. Redirected
        edges follow _weld_into; surviving faces keep their keys."""
        self._chunk_dirty = True
        self._mut_serial += 1
        from ingetrazo_core import weld_object_mesh
        return weld_object_mesh(self, _key, 10.0 ** -_KEY_DECIMALS)

    def dedupe_faces(self) -> int:
        """Drop faces occupying the same *outer* edge cycle as another — a box
        pushed flat (top welded onto bottom), or a room raised whose shared
        wall the neighbour's push already built. Two faces over one region are
        always junk in a surface model; only one is kept. When the pair
        differs in holes (a plain cap collapsed onto a subdivided base), the
        one with more holes survives — it carries the user's subdivision, and
        its holes are filled by their own faces. Returns how many were
        removed. The face with more holes keeps its key; ties keep the
        first face in mesh order. Removed faces retire their keys."""
        self._chunk_dirty = True
        self._mut_serial += 1
        seen: dict = {}
        removed = 0
        for f in list(self.faces):
            sig = frozenset(
                frozenset((f.loop[i].key, f.loop[(i + 1) % len(f.loop)].key))
                for i in range(len(f.loop))
            )
            other = seen.get(sig)
            if other is None:
                seen[sig] = f
                continue
            drop, keep = ((other, f) if len(f.hole_loops) > len(other.hole_loops)
                          else (f, other))
            if drop.attrs and not keep.attrs:
                keep.attrs = dict(drop.attrs)
            self.remove_face(drop)
            seen[sig] = keep
            removed += 1
        return removed

    def _weld_into(self, keep: Vertex, dup: Vertex) -> None:
        """Keep ``keep``'s key and retire ``dup``. Surviving faces keep keys.

        An existing destination edge keeps its key; otherwise the redirected
        edge keeps its old key. Zero-length edges and sliver faces retire.
        """
        from ingetrazo_core import weld_object_vertex
        return weld_object_vertex(self, keep, dup, _key)

    def interior_vertex_on(self, edge: Edge) -> Optional[Vertex]:
        """An existing vertex lying strictly on ``edge``'s interior (a
        T-junction: ``edge`` runs past a vertex that belongs to another face),
        or ``None``. Splitting there is what makes mismatched subdivisions share
        connectivity instead of leaving a naked seam."""
        a = edge.v0.position
        b = edge.v1.position
        ab = b - a
        length = ab.length()
        if length < _STITCH_TOL:
            return None
        for v in self.vertices:
            if v == edge.v0 or v == edge.v1:
                continue
            t = QVector3D.dotProduct(v.position - a, ab) / (length * length)
            if t <= _STITCH_TOL / length or t >= 1.0 - _STITCH_TOL / length:
                continue
            if (v.position - (a + ab * t)).length() < _STITCH_TOL:
                return v
        return None

    def split_edge_at(self, edge: Edge, mid: Vertex) -> dict:
        """Split ``edge`` at the *existing* vertex ``mid`` on its interior.
        Returns a reversible record consumed by :meth:`revert_stitch`.
        A new first half keeps the original key; existing halves keep theirs.
        A new second half gets a fresh key. Faces and vertices keep keys.

        A sub-edge may already exist — the T-junction's other face owns it — in
        which case :meth:`_link_edge` reuses it. The record tracks which sub-edges
        this split *created* so undo removes only those and merely detaches the
        face from the reused ones (never deleting a neighbour's edge)."""
        from ingetrazo_core import split_object_edge
        return split_object_edge(self, edge, mid)

    def collapsible_vertex(self, v: Vertex) -> bool:
        """Whether ``v`` is a redundant valence-2 collinear vertex: its two edges
        are collinear and border the same faces, so it is a spurious subdivision
        of a straight boundary (no edge or face needs it); a surface model keeps
        no such vertex. Removing it lets two coplanar faces share a single edge again."""
        if len(v.edges) != 2:
            return False
        e1, e2 = list(v.edges)
        d1 = v.position - e1.other(v).position
        d2 = e2.other(v).position - v.position
        if d1.length() < _STITCH_TOL or d2.length() < _STITCH_TOL:
            return False
        if QVector3D.dotProduct(d1.normalized(), d2.normalized()) < 0.999:
            return False
        if set(e1.faces) != set(e2.faces):
            return False
        # A loop of three keeps its corners, however thin: a sliver hole in
        # an STL import lost one and became a hole of two, and the model
        # could no longer be saved (IndexError in _maximal_holes; Windows
        # round 0.6.2, already in 0.6.1).
        return not any(len(loop) <= 3 and v in loop
                       for face in e1.faces
                       for loop in (face.loop, *face.hole_loops))

    def collapse_vertex(self, v: Vertex) -> dict:
        """Remove the redundant valence-2 collinear vertex ``v`` (see
        :meth:`collapsible_vertex`), merging its two edges into one. Returns a
        reversible record. Surviving faces/endpoints keep their keys. An
        existing joined edge keeps its key; a new joined edge gets a fresh
        key, retiring both old edges and the removed vertex. A new joined
        edge runs from the end with the smaller position (as a tuple) to the
        other."""
        e1, e2 = list(v.edges)
        n1, n2 = e1.other(v), e2.other(v)
        # The joined edge runs from the end with the smaller position: the
        # two edges came in set order (object addresses), so its direction
        # differed from run to run (design decision 5). Faces in e1's order.
        if n2.position.toTuple() < n1.position.toTuple():
            e1, e2, n1, n2 = e2, e1, n2, n1
        faces = list({f.key: f for f in e1.faces}.values())
        self.remove_edge(e1)
        self.remove_edge(e2)
        new_edge_existed = self.find_edge(n1, n2) is not None
        new_edge = self._link_edge(n1, n2)
        for face in faces:
            for loop in (face.loop, *face.hole_loops):
                if v in loop:
                    loop.remove(v)
            if face not in new_edge.faces:
                new_edge.faces.append(face)
        if v in self.vertices:
            self.vertices.remove(v)
        if self._registry.get(_key(v.position)) is v:
            del self._registry[_key(v.position)]
        return {"kind": "collapse", "v": v, "e1": e1, "e2": e2, "n1": n1,
                "n2": n2, "new_edge": new_edge, "new_edge_new": not new_edge_existed,
                "faces": faces}

    def collapse_collinear(self) -> int:
        """Phase 2 of ``core.history.run_stitch``: while a listed vertex is
        :meth:`collapsible_vertex`, collapse the first one in list order.
        Returns how many went. A method so a store-backed mesh does it in
        one native pass (``StoreMesh.collapse_collinear``)."""
        n = 0
        while True:
            for v in list(self.vertices):
                if self.collapsible_vertex(v):
                    self.collapse_vertex(v)
                    n += 1
                    break
            else:
                return n

    def dissolve_edge_recorded(self, edge: Edge) -> Optional[dict]:
        """:meth:`dissolve_edge`, but returning a reversible record (the removed
        faces, the edge, and the merged face) instead of just the new face.
        The merged face gets a fresh key; both old faces and the shared edge
        retire. Other edges and vertices keep their keys."""
        from ingetrazo_core import dissolve_object_edge
        return dissolve_object_edge(self, edge, True, _dominant_attrs)

    def dissolve_coplanar_pair(self, face_a: "Face", face_b: "Face") -> Optional[dict]:
        """Merge two coplanar faces into the boundary of their union, removing
        *every* edge they share — not just one. T-junction resolution can leave
        two coplanar faces sharing several edges (a footprint notch), which the
        single-edge splice can't fuse. Traces the union outline instead. Returns
        a reversible record, or ``None`` if they aren't a coplanar adjacent pair
        with a simple union outline."""
        from ingetrazo_core import dissolve_object_region
        return dissolve_object_region(self, [face_a, face_b], True, _trace_loops,
                                      _loop_area, _orient_union_loops, _dominant_attrs)

    def dissolve_coplanar_region(self, faces: Iterable["Face"]) -> Optional["Face"]:
        """Merge a set of mutually-coplanar, edge-connected faces into one face
        spanning the boundary of their union, removing every interior edge.

        Pairwise merging is order-dependent — fusing two of three faces can leave
        a valence-3 junction the third can't bridge — so the stitch merges the
        whole coplanar component at once, which is deterministic. ``None`` if the
        faces aren't all coplanar, aren't connected, or the union isn't simple."""
        faces = list(faces)
        from ingetrazo_core import dissolve_object_region
        return dissolve_object_region(self, faces, False, _trace_loops,
                                      _loop_area, _orient_union_loops, _dominant_attrs)

    def revert_stitch(self, rec: dict) -> None:
        """Undo a single stitch primitive recorded by the methods above."""
        kind = rec["kind"]
        if kind == "split":
            from ingetrazo_core import revert_object_split
            revert_object_split(self, rec)
        elif kind == "collapse":
            v = rec["v"]
            if v not in self.vertices:
                self.vertices.append(v)
            self._registry.setdefault(_key(v.position), v)
            if rec["new_edge_new"]:
                self.remove_edge(rec["new_edge"])
            else:
                for face in rec["faces"]:
                    if face in rec["new_edge"].faces:
                        rec["new_edge"].faces.remove(face)
            self.relink_edge(rec["e1"])
            self.relink_edge(rec["e2"])
            for face in rec["faces"]:
                self._insert_into_face_loops(face, rec["n1"], rec["n2"], v)
                if face not in rec["e1"].faces:
                    rec["e1"].faces.append(face)
                if face not in rec["e2"].faces:
                    rec["e2"].faces.append(face)
        elif kind in ("merge", "merge_pair"):
            from ingetrazo_core import revert_recorded_union
            revert_recorded_union(self, rec, kind == "merge_pair")

    def dissolve_edge(self, edge: Edge) -> Optional[Face]:
        """Dissolve a *redundant* coplanar edge, merging its two faces into one.

        Coplanar merge: an edge bordering exactly two faces that lie
        in the same plane (same outward normal) carries no silhouette — it is the
        seam left when a pushed wall ends up flush with an adjacent one. Splicing
        the two face loops along the shared edge and dropping the edge yields a
        single face (the "L"), no phantom line.

        Returns the new merged face, or ``None`` if the edge is not mergeable
        (not exactly two faces, not coplanar, shared on a hole loop, or the
        splice would not be a simple loop). The radial model makes this cheap:
        the two incident faces are right there on ``edge.faces``; nothing is
        rediscovered by position.
        """
        from ingetrazo_core import dissolve_object_edge
        return dissolve_object_edge(self, edge, False, _dominant_attrs)



def _deferred_state(name):
    def get(self):
        self._materialize()
        return getattr(self, name)

    def put(self, value):
        self._materialize()
        setattr(self, name, value)
    return property(get, put)


# During an agent transaction, take the undo snapshot on first touch, once
# the original elements exist and before the operation can change them.
on_mesh_materialized = ContextVar("on_mesh_materialized", default=None)


class DeferredMesh(Mesh):
    """A mesh read from a document whose vertices, edges and faces are not
    built yet (the experimental engine's lazy meshes): the first touch of
    any of them builds them, with the very code an eager read runs, and
    turns the object back into a plain :class:`Mesh`. A big model whose
    groups are only looked at never pays for millions of Python objects.

    :meth:`defer` only takes an EMPTY mesh, so the deferred build welds
    against the same empty registry the eager one would."""

    vertices = _deferred_state("vertices")
    edges = _deferred_state("edges")
    faces = _deferred_state("faces")
    _registry = _deferred_state("_registry")
    _displaced = _deferred_state("_displaced")

    @staticmethod
    def defer(mesh: Mesh, build, nfaces: int, source=None) -> bool:
        """Turn ``mesh`` into a deferred one that runs ``build(mesh)`` on
        first touch and makes exactly ``nfaces`` faces; ``source`` is what
        it is built from, for :mod:`core.lazy_mesh` (kept until built).
        False (nothing done) unless it is an empty object or store mesh."""
        d = mesh.__dict__
        from core.store_mesh import StoreMesh
        if type(mesh) is StoreMesh:
            if any(mesh._store.counts()):
                return False
            for name in _STORE_SIDE:
                d.pop(name, None)
        elif (type(mesh) is not Mesh or d["vertices"] or d["edges"]
                or d["faces"] or d["_registry"]):
            return False
        d["_deferred"] = build
        d["_deferred_nfaces"] = nfaces
        d["_deferred_source"] = source
        mesh.__class__ = DeferredMesh
        return True

    def _materialize(self) -> None:
        """First touch: a store-backed mesh when the store is on
        (``core.store_mesh.from_deferred``), else the Python objects the
        loader's build makes."""
        from core import store_mesh
        if not store_mesh.from_deferred(self):
            self._build_python_mesh()
        observe = on_mesh_materialized.get()
        if observe is not None:
            observe(self)

    def _build_python_mesh(self):
        self.__dict__.pop("_deferred_hooks", None)
        self.__dict__.pop("_deferred_hook_source", None)
        build = self.__dict__.pop("_deferred")
        del self.__dict__["_deferred_nfaces"]
        del self.__dict__["_deferred_source"]
        self.__dict__.pop("_deferred_plan", None)
        self.__dict__.pop("_deferred_geom", None)
        self.__dict__.pop("_drawing_staged", None)
        self.__dict__.pop("_drawing_source_key", None)
        self.__dict__.pop("_drawing_texture_paths", None)
        self.__class__ = Mesh
        if _store_on():
            from core.store_mesh import initialise_store
            initialise_store(self)
        elif "vertices" not in self.__dict__:
            self._init_object_geometry()
        build(self)

    def __reduce_ex__(self, protocol):
        # deepcopy / pickle copy the built mesh, never the pending build.
        self._materialize()
        return self.__reduce_ex__(protocol)


def _first_touch(name):
    """A Mesh method called on a still-deferred mesh: build it (a store or
    Python objects), then run the method of what it became."""
    def method(self, *args, **kw):
        # Python binds m.move_vertex before evaluating m.vertices[0]. The
        # argument can therefore have materialised this very mesh already.
        if isinstance(self, DeferredMesh):
            self._materialize()
        return getattr(self, name)(*args, **kw)
    method.__name__ = name
    return method


# Every Mesh method that reads or writes the geometry first-touches it, so
# a method never starts as a DeferredMesh and goes on as something else.
for _name, _value in list(Mesh.__dict__.items()):
    if (callable(_value) and not isinstance(_value, staticmethod)
            and _name not in ("__init__", "__deepcopy__", "bump_revision",
                              "mark_modified", "invalidate_placement_chunks")
            and not (_name.startswith("__") and _name.endswith("__"))):
        setattr(DeferredMesh, _name, _first_touch(_name))
del _name, _value


def face_count(mesh) -> int:
    """``len(mesh.faces)``, without building a deferred mesh to learn it."""
    if type(mesh) is DeferredMesh:
        return mesh.__dict__["_deferred_nfaces"]
    return len(mesh.faces)


def face_attrs(mesh):
    """Each face's own attrs, in face order — for a deferred or store-backed
    mesh only the first face of each distinct set (what a first-wins tally
    of materials needs), read without building Python objects."""
    if type(mesh) is DeferredMesh:
        import numpy as np
        from formats.igz import _face_attrs_from_json
        src = mesh.__dict__["_deferred_source"]
        table = src["meta"].get("face_attrs") or []
        fa = np.asarray(src["face_attr"], dtype=np.int64)
        _u, first = np.unique(fa, return_index=True)
        for k in fa[np.sort(first)].tolist():
            yield (_face_attrs_from_json(table[k]) or {}) if k >= 0 else {}
        return
    from core import store_mesh
    if type(mesh) is store_mesh.StoreMesh:
        yield from store_mesh.face_attrs(mesh)
        return
    for f in mesh.faces:
        yield f.attrs


def turn_face_over(face) -> None:
    """Reverse ``face``'s winding for the automatic orientation passes —
    which must not move paint from one side of a wall to the other.

    Paint lives by side: the front's in ``attrs`` (``PAINT_KEYS``), the
    back's own in ``attrs["back"]`` (a dict), or ``True`` when the back
    mirrors the front. Reversing the loop alone moves "front" to the other
    geometric side and takes its paint along. When the BACK has paint of
    its own, the sides were painted deliberately — a house wall, brick
    outside and plaster inside, wound the wrong way — and the two paints
    trade places as the winding turns, so the street keeps its brick and
    only the face now points the right way (Marco's question, 2026-09-18).

    A face painted on ONE side keeps the old rule: the paint follows the
    front. That is what a painted rectangle pulled into a box relies on —
    the base, turned over to face down, must come out painted OUTSIDE, «a
    painted box, not a box with one painted face» (Marco, 2026-08-27). The
    two rules meet where they should: paint on both sides pins the sides;
    paint on one side goes with the correction.

    The explicit Reverse Faces command (``FlipFacesCommand``) swaps nothing,
    by design: there the material follows the front, the classic behaviour."""
    swap_orientation_paint(face)
    face.reverse_rings()


def swap_orientation_paint(face) -> None:
    """Keep explicit front and back paint on their geometric sides at a flip."""
    attrs = face.attrs if face.attrs is not None else {}
    back = attrs.get("back")
    if isinstance(back, dict):
        front = {k: attrs.pop(k) for k in PAINT_KEYS if k in attrs}
        attrs.pop("back", None)
        attrs.update(back)
        if front:
            attrs["back"] = front
        face.attrs = attrs


def _dominant_attrs(faces) -> dict:
    """The attrs of the largest-area face that carries any — the dominant
    contributor when a merge collapses several faces into one survivor."""
    best: dict = {}
    best_area = -1.0
    for f in faces:
        if f.attrs and f.area() > best_area:
            best = f.attrs
            best_area = f.area()
    return best


def _trace_loops(outline: list[tuple[Vertex, Vertex]]) -> Optional[list[list[Vertex]]]:
    """Trace a set of undirected boundary edges into closed vertex loops. Returns
    ``None`` if any vertex has valence ≠ 2 (a pinched / non-simple outline).

    Each loop starts at the first outline segment no loop has walked yet,
    in that segment's direction: the start was the first member of a set
    of segments, i.e. object addresses, so the merged face's corner order
    differed from run to run (design decision 5). Vertices are known by
    key, so the store's views trace the same loops."""
    from ingetrazo_core import trace_object_loops
    return trace_object_loops(outline)


def _orient_union_loops(
    outer: list[Vertex], holes: list[list[Vertex]], normal: QVector3D
) -> None:
    """Restore face winding after tracing an undirected union boundary.

    The tracer may choose either direction independently for each loop.
    Keep the source face's front side and wind holes opposite to the outer
    boundary, without swapping front/back paint.
    """
    if QVector3D.dotProduct(Face(outer).normal(), normal) < 0:
        outer.reverse()
    for hole in holes:
        if QVector3D.dotProduct(Face(hole).normal(), normal) > 0:
            hole.reverse()


def _loop_area(loop: list[Vertex]) -> float:
    """Newell-magnitude area of a vertex loop (orientation-independent)."""
    n = QVector3D(0.0, 0.0, 0.0)
    count = len(loop)
    for i in range(count):
        cur = loop[i].position
        nxt = loop[(i + 1) % count].position
        n = n + QVector3D(
            (cur.y() - nxt.y()) * (cur.z() + nxt.z()),
            (cur.z() - nxt.z()) * (cur.x() + nxt.x()),
            (cur.x() - nxt.x()) * (cur.y() + nxt.y()),
        )
    return 0.5 * n.length()


def element_identity(item):
    """Identity for heterogeneous UI lists and legacy geometry planners.

    Mesh elements use stable keys; groups, Qt items and temporary legacy
    geometry still use Python identity.
    """
    return item.key if isinstance(item, (Vertex, Edge, Face)) else id(item)
