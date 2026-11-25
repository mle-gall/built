# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Public mesh and element types for the shared-vertex geometry model.

Mesh owns a native store. Vertex, Edge and Face are also constructible as
detached elements. Store views implement the same interfaces and resolve their
geometry and incidence by key.
"""
from __future__ import annotations

from contextvars import ContextVar
from typing import Iterable, Optional
from itertools import count

from ingetrazo_core import object_drawing, object_replay, Vec3 as QVector3D, polygon_centroid, polygon_net_area, polygon_newell


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
_WELD_TOL2 = (10.0 ** -_KEY_DECIMALS) ** 2


def _key(p: QVector3D) -> tuple[int, int, int]:
    # Integer grid cells (same partition as rounding to _KEY_DECIMALS):
    # neighbouring cells are ±1, so the boundary probes in _lookup and
    # weld_coincident need no float rounding — the rounding calls dominated
    # large imports.
    return object_replay.mesh_key(p)



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
        return object_replay.vertex_faces(self.edges)

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
        object_replay.reverse_rings(self)

    # ---- Legacy-compatible read interface (positions) -----------------------
    @property
    def vertices(self) -> list[QVector3D]:
        return [v.position for v in self.loop]

    @property
    def holes(self) -> list[list[QVector3D]]:
        return [[v.position for v in h] for h in self.hole_loops]

    # ---- Geometry (ported from legacy Face, over shared positions) ----------
    def _newell(self) -> QVector3D:
        return polygon_newell(self.vertices)

    def normal(self) -> QVector3D:
        return object_replay.ring_normal(self.loop)

    def area(self) -> float:
        return object_replay.loop_area(self.loop)

    def net_area(self) -> float:
        return polygon_net_area(self.vertices, self.holes)

    def centroid(self) -> QVector3D:
        return polygon_centroid(self.vertices)

    def triangulate(self, normal: QVector3D | None = None
                    ) -> list[tuple[QVector3D, QVector3D, QVector3D]]:
        if len(self.loop) < 3:
            return []
        from core.triangulate import triangulate

        return triangulate(self.vertices, self.holes,
                           normal if normal is not None else self.normal())

    def __repr__(self) -> str:  # pragma: no cover - debug aid
        return f"Face({len(self.loop)} verts, {len(self.holes)} holes)"


# During an agent transaction, take the undo snapshot on first touch, once
# the original elements exist and before the operation can change them.
on_mesh_materialized = ContextVar("on_mesh_materialized", default=None)


class _MeshState:
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

    @staticmethod
    def next_curve_id() -> int:
        """A fresh curve id — for tools that re-create curve edges (paste,
        offset) and must give the copy its own contour identity."""
        global _CURVE_COUNTER
        cid = _CURVE_COUNTER
        _CURVE_COUNTER += 1
        return cid

    def note_rewound(self) -> None:
        """Say that faces may have changed winding (#197): no count,
        coordinate or paint shows it, so the drawing's chunk has to be told.
        Marked dirty too, or a move in the same breath would carry the old
        triangles along on the translation fast path."""
        self._winding_rev = getattr(self, "_winding_rev", 0) + 1
        self._chunk_dirty = True
        self.__dict__["_attrs_dirty_value"] = True   # one revision bump

    def add_faces_bulk(self, pos, ring_sizes, face_ring_counts, attrs=None):
        """Convenience: :meth:`bulk_weld` + :meth:`add_faces_welded`."""
        vobjs, inverse = self.bulk_weld(pos)
        return self.add_faces_welded(vobjs, inverse, ring_sizes,
                                     face_ring_counts, attrs)


def __getattr__(name):
    # Resolve the public types lazily: the store views extend the element types above.
    if name in ("Mesh", "DeferredMesh"):
        from core.store_mesh import DeferredMesh, StoreMesh
        return StoreMesh if name == "Mesh" else DeferredMesh
    raise AttributeError(name)


def face_count(mesh) -> int:
    """``len(mesh.faces)``, without building a deferred mesh to learn it."""
    from core.store_mesh import DeferredMesh
    if type(mesh) is DeferredMesh:
        return mesh.__dict__["_deferred_nfaces"]
    return len(mesh.faces)


def face_attrs(mesh):
    """Each face's own attrs, in face order — for a deferred or store-backed
    mesh only the first face of each distinct set (what a first-wins tally
    of materials needs), read without building Python objects."""
    from core.store_mesh import DeferredMesh
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
    from core.store_mesh import StoreMesh
    if isinstance(mesh, StoreMesh):
        yield from store_views.attribute_rows(mesh)
        return
    for f in mesh.faces:
        yield f.attrs


from ingetrazo_core import store_views

turn_face_over = store_views.turn_face_over
swap_orientation_paint = store_views.swap_orientation_paint


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


def element_identity(item):
    """Identity for heterogeneous UI lists and legacy geometry planners.

    Mesh elements use stable keys; groups, Qt items and temporary legacy
    geometry still use Python identity.
    """
    return item.key if isinstance(item, (Vertex, Edge, Face)) else id(item)


__all__ = [name for name in globals() if not name.startswith("_")] + ["Mesh", "DeferredMesh"]
