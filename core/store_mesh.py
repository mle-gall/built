# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Python mesh interfaces backed by the native MeshStore.

Deferred document blocks build on first geometry access. Element views
resolve keys against the active coordinate frame. StoreState shares native
snapshot chunks and preserves per-face Python attributes."""
from __future__ import annotations

from collections.abc import Mapping
from functools import partial

import numpy as np
from ingetrazo_core import Vec3 as QVector3D, MeshElements, StoreFallback, polygon_centroid, polygon_net_area, polygon_newell as _newell, store_views

from core import mesh as _mesh
from core.mesh import Edge, Face, Vertex

LOCAL_MASK = (1 << 40) - 1

#: ``mesh.__dict__`` entries that belong to the store side and never travel
#: with a deep copy of the Python state.
STORE_SIDE = ("_store", "_attr_table", "_attr_own", "_bulk",
              "_pos_cache", "_face_cache", "_frame_parent",
              "_tool_frame_cache", "_planned_points", "_instance_definition")


def _weld_args():
    """Use the same unfused squared distance as the binary64 model vector."""
    return dict(scale=_mesh._KEY_SCALE,
                tol2=(10.0 ** -_mesh._KEY_DECIMALS) ** 2,
                fused=False)


# ---- Views ----------------------------------------------------------------

def _attribute_owner(mesh):
    """The facade holding this definition's face attribute rows."""
    return mesh.__dict__.get("_instance_definition", mesh).__dict__


def _store_of(m):
    """The live store of a store-backed mesh, else None."""
    return m.__dict__.get("_store")


class _ViewMixin:
    __slots__ = ()

    def __init__(self, m, key):
        self._m = m
        self.key = key

    def __eq__(self, other):
        if other is self:
            return True
        if isinstance(other, (Vertex, Edge, Face)):
            return other.key == self.key
        return NotImplemented

    def __ne__(self, other):
        r = self.__eq__(other)
        return r if r is NotImplemented else not r

    def __hash__(self):
        return hash(self.key)

    def __deepcopy__(self, memo):
        from copy import deepcopy
        if not self._m._store.knows(self.key):
            raise KeyError(f"unknown mesh element {self.key}")
        owner = deepcopy(self._m, memo)
        key = (owner._store.namespace << 40) | (self.key & LOCAL_MASK)
        return type(self)(owner, key)

    def __reduce__(self):
        return _unpickle_view, (type(self), self._m, self.key & LOCAL_MASK)


def _unpickle_view(cls, mesh, local_key):
    return cls(mesh, mesh._store.key_base | local_key)


class VertexView(_ViewMixin, Vertex):
    __slots__ = ("_m",)

    @property
    def position(self):
        m = self._m
        return store_views.position(m, self.key)

    def position_tuple(self):
        """Model coordinates for geometry consumers, including sub-float32 edits."""
        st = self._m._store
        return tuple(st.vertex_position(self.key))

    @position.setter
    def position(self, value):
        self._m.place_vertex(self, value)

    edges = property(store_views.vertex_edges, store_views.set_vertex_edges)


class EdgeView(_ViewMixin, Edge):
    __slots__ = ("_m",)

    v0 = property(partial(store_views.endpoint, 0), partial(store_views.set_endpoint, 0))
    v1 = property(partial(store_views.endpoint, 1), partial(store_views.set_endpoint, 1))

    faces = property(store_views.edge_faces, store_views.set_edge_faces)

    soft = property(partial(store_views.edge_flag, "soft"), partial(store_views.set_edge_flag, "soft"))
    hidden = property(partial(store_views.edge_flag, "hidden"), partial(store_views.set_edge_flag, "hidden"))
    flat = property(partial(store_views.edge_flag, "flat"), partial(store_views.set_edge_flag, "flat"))
    curve = property(partial(store_views.edge_flag, "curve"), partial(store_views.set_edge_flag, "curve"))
    layer = property(partial(store_views.edge_flag, "layer"), partial(store_views.set_edge_flag, "layer"))

    @property
    def a(self) -> QVector3D:
        m = self._m
        k = m.__dict__["_store"].edge_ends(self.key)[0]
        return store_views.position(m, k)

    @property
    def b(self) -> QVector3D:
        m = self._m
        k = m.__dict__["_store"].edge_ends(self.key)[1]
        return store_views.position(m, k)

    def other(self, v):
        """The endpoint that is not ``v`` (by key: views are never
        identical)."""
        v0 = self.v0
        return self.v1 if v.key == v0.key else v0

    def has(self, v) -> bool:
        return v.key == self.v0.key or v.key == self.v1.key

    def length(self) -> float:
        st = self._m._store
        a, b = st.edge_ends(self.key)
        from math import dist
        return dist(st.vertex_position(a), st.vertex_position(b))


class FaceView(_ViewMixin, Face):
    __slots__ = ("_m",)

    loop = property(partial(store_views.face_loops, False), partial(store_views.set_face_loops, False))
    hole_loops = property(partial(store_views.face_loops, True), partial(store_views.set_face_loops, True))

    def _positions(self):
        return store_views.ring_positions(self._m, self.key)

    def _vectors(self):
        return store_views.ring_vectors(self._m, self.key)

    @property
    def vertices(self):
        rings = self._vectors()
        return list(rings[0])

    @property
    def holes(self):
        rings = self._vectors()
        return [list(r) for r in rings[1:]]

    interior = property(store_views.interior, store_views.set_interior)
    attrs = property(store_views.face_attrs, store_views.set_face_attrs)

    def reverse_rings(self) -> None:
        """``turn_face_over``'s ``loop.reverse()`` and holes' ``reverse()``
        as one store op: a push/pull turning its base over no longer
        materialises the mesh. Python bumps no counter for it, neither
        does this (the store's own revision moves, for the view caches)."""
        st = self._m._store
        st.reverse_face(self.key)

    # Geometry from one fetch of the rings' positions.
    def _newell(self) -> QVector3D:
        rings = self._positions()
        return _newell(rings[0])

    def triangulate(self, normal=None):
        from core._native import qt_hypot
        store = self._m._store
        if qt_hypot() and store.is_live(self.key):
            triangles = store.face_triangles(self.key, qt_hypot(),
                                             normal.toTuple() if normal is not None else None)
            return [tuple(QVector3D(*p) for p in t) for t in triangles]
        return Face.triangulate(self, normal)

    def normal(self):
        return store_views.cached_normal(self._m, self.key)

    def area(self) -> float:
        rings = self._positions()
        return store_views.face_area(rings[0])

    def net_area(self) -> float:
        rings = self._positions()
        return polygon_net_area(rings[0], rings[1:])

    def centroid(self) -> QVector3D:
        rings = self._positions()
        return polygon_centroid(rings[0])


_BASE = {VertexView: Vertex, EdgeView: Edge, FaceView: Face}


class _MutList(list):
    """A write-through copy of a face ring, its holes, or an edge’s face order."""
    def __init__(self, owner, path, items):
        super().__init__(items)
        self._owner, self._path = owner, path

    def _write(name):
        def method(self, *args, **kwargs):
            field, *index = self._path
            current = getattr(self._owner, field)
            if index and (index[0] >= len(current) or current[index[0]] != self):
                raise ValueError("the held hole ring has been replaced")
            target = list(current[index[0]] if index else current)
            result = getattr(target, name)(*args, **kwargs)
            if index:
                rings = list(current)
                rings[index[0]] = target
                setattr(self._owner, field, rings)
            else:
                setattr(self._owner, field, target)
            if field == "hole_loops" and not index:
                target = self._owner.hole_loops
            list.__setitem__(self, slice(None), target)
            return result
        return method

    append = _write("append")
    clear = _write("clear")
    extend = _write("extend")
    insert = _write("insert")
    pop = _write("pop")
    remove = _write("remove")
    reverse = _write("reverse")
    sort = _write("sort")
    __setitem__ = _write("__setitem__")
    __delitem__ = _write("__delitem__")

    def __iadd__(self, items):
        self.extend(items)
        return self

    _repeat = _write("__imul__")

    def __imul__(self, n):
        self._repeat(n)
        return self


class _MutSet(set):
    def _readonly(self, *args):
        raise ValueError("vertex.edges is determined by the endpoints of live edges")

    add = discard = remove = pop = clear = update = _readonly
    difference_update = intersection_update = symmetric_difference_update = _readonly
    __ior__ = __iand__ = __ixor__ = __isub__ = _readonly


# ---- Snapshots --------------------------------------------------------------

class StoreState(store_views.MeshSnapshotRecord, Mapping):
    """``capture_state()`` of a store-backed mesh: the store's snapshot
    (O(chunks)), the faces' own attrs dicts (copied, as Python copies
    ``f.attrs``) and the attribute table they index.

    It reads like the Python snapshot dict for the few callers that peek
    (``snap["vpos"]`` for "was this vertex there", ``snap["faces"]`` /
    ``["edges"]`` as elements of the mesh); every other entry is built on
    demand from the snapshot's arrays."""

    __slots__ = ("snap", "own", "table", "mesh", "_cache", "owner")

    def __init__(self, snap, own, table, mesh):
        self.snap = snap
        self.owner = mesh._store
        self.own = own
        self.table = table
        self.mesh = mesh
        self._cache = {}

    # -- the Python snapshot's entries
    _KEYS = ("vertices", "edges", "faces", "registry", "vpos", "vedges",
             "efaces", "esoft", "eflat", "ehidden", "elayer", "ecurve", "floop",
             "fholes", "fattrs", "finterior")

class _SnapPositions(Mapping):
    """``snap["vpos"]``: vertex key → position in the snapshot."""

    __slots__ = ("snap",)

    def __init__(self, snap):
        self.snap = snap

    def __contains__(self, key):
        return isinstance(key, int) and self.snap.has(key) and \
            (key >> 38) & 3 == 0

    def __getitem__(self, key):
        try:
            return QVector3D(*self.snap.vertex_position(key))
        except (KeyError, OverflowError, TypeError):
            raise KeyError(key) from None

    def __iter__(self):
        return iter(self.snap.keys("vertex"))

    def __len__(self):
        return self.snap.counts()[0]


# ---- The store-backed mesh -------------------------------------------------

def _seq_property(kind, cls):
    def get(self):
        return MeshElements(self, kind, cls, _BASE[cls])

    def put(self, value):
        get(self).replace(value)
    return property(get, put)


class StoreMesh(_mesh._MeshState):
    """The public Mesh facade over a native geometry store."""

    def __init__(self):
        self._mut_serial = 0
        self._chunk_dirty = True
        store_views.initialise_store(self)

    vertices = _seq_property("vertex", VertexView)
    edges = _seq_property("edge", EdgeView)
    faces = _seq_property("face", FaceView)
    @property
    def _registry(self):
        cells, indices = self._store.registry()
        vertices = self.vertices
        return {tuple(cell): vertices[i] for cell, i in zip(cells.tolist(), indices.tolist())}

    @property
    def _displaced(self):
        return {cell: [VertexView(self, k) for k in keys]
                for cell, keys in self._store.displaced_keys().items()}

    def curve_edges(self, edge):
        cid = edge.curve
        if cid is None:
            return [edge]
        return [EdgeView(self, k) for k in self._store.curve_keys(cid)]

    def grow_pick(self, entity, grow):
        result = self._store.tool_selection(entity.key, grow)
        if result is None:
            return None
        edges, faces = result
        # Natural growth retains its seed face (or a single, uncurved edge).
        # A connected walk returns the elements reached through incidence.
        seed = entity.key if grow != "connected" and (
            isinstance(entity, Face) or entity.curve is None) else None
        return ([entity if key == seed else EdgeView(self, key) for key in edges]
                + [entity if key == seed else FaceView(self, key) for key in faces])

    def surface_of(self, face):
        result = self.grow_pick(face, "natural")
        return [face] if result is None else result

    # -- what the store answers itself
    def _bump(self):
        self._chunk_dirty = True
        self._mut_serial += 1
        self.__dict__.pop("_bulk", None)

    def _input_point(self, position):
        xyz = _xyz(position)
        planned = self.__dict__.get('_planned_points')
        return planned.get(xyz, xyz) if planned else xyz

    def vertex_at(self, position):
        k = self._store.vertex_at(self._input_point(position))
        return None if k is None else VertexView(self, k)

    def _lookup(self, position):
        return self.vertex_at(position)

    def registers(self, v) -> bool:
        try:
            return self._store.registers(v.key)
        except (OverflowError, TypeError):
            return False

    def find_edge(self, v0, v1):
        if v0 is None or v1 is None:
            return None
        try:
            k = self._store.find_edge(v0.key, v1.key)
        except (KeyError, OverflowError, TypeError):
            return None
        return None if k is None else EdgeView(self, k)

    def edges_at(self, v):
        return set(v.edges)

    def faces_at(self, v):
        return v.faces()

    def move_vertex(self, v, delta) -> None:
        self._bump()
        self._store.move_vertex(v.key, _xyz(delta))

    def place_vertex(self, v, position) -> None:
        self._bump()
        self._store.place_vertex(v.key, _xyz(position))

    def vertices_in_cells(self, cells, keyfn) -> list:
        """Views of the vertices with ``keyfn(position) in cells``, in mesh
        order (``history._vertices_at``), ``keyfn`` rounding to 4 decimals
        (``topology._key``: floats; ``mesh._key``: cells x 10^4). Candidates
        from the position array (a cell's points floor to it or the cell
        below), then ``keyfn`` itself on those: Python's answer exactly."""
        return store_views.vertices_in_cells(self, self._store, cells, keyfn, VertexView)

    def cells_of(self, vertex_keys, keyfn) -> set:
        return store_views.cells_of(self._store, vertex_keys, keyfn)

    def textured_faces(self) -> list:
        """The faces whose attrs hold a texture (own dict or table row),
        without giving the others a dict."""
        table, fa = store_views.attr_columns(self)
        rows = {i for i, a in enumerate(table) if a and a.get("texture")}
        if not rows:
            return []
        keys = self._store.keys("face")
        return [FaceView(self, keys[i]) for i, r in enumerate(fa.tolist())
                if r in rows]

    def nonplanar_faces(self, tolerance):
        """The faces ``fold_nonplanar_faces`` folds, found by the store
        (``is_planar`` ported exactly): a Move on a big store-backed mesh
        no longer reads every face through a view."""
        return [FaceView(self, k)
                for k in self._store.nonplanar_faces(tolerance)]

    # -- port step 2: the incidence primitives as store ops ---------------
    # Each is its Mesh method's semantics (store/topo.rs), with the same
    # dirty / revision bumps on the Python side (`_dirty(n)` = n times
    # ``_chunk_dirty = True; _mut_serial += 1``), so readers keyed on them
    # see the same thing. An element of another mesh, or a result the
    # store does not represent (``StoreFallback``, raised before anything
    # changed), runs the Python kernel instead (`_py`).

    def _dirty(self, n=1) -> None:
        return store_views.dirty(self, n)

    def _inherit_face_attrs(self, dirty, replacements):
        """Transfer editor attributes to native replacement pieces and return their source faces."""
        if dirty:
            self._dirty(dirty)
        sources = []
        for old, pieces in replacements:
            source = FaceView(self, old)
            sources.append(source)
            for key in pieces:
                FaceView(self, key).attrs = dict(source.attrs)
        return sources

    def _own(self, x, kind):
        return store_views.owned(self, x, kind)

    def _incidence_key(self, element, kind):
        expected = (VertexView, EdgeView, FaceView)[kind]
        if not isinstance(element, expected) or self._live(element, kind) is None:
            raise ValueError(f"incidence requires a live {('vertex', 'edge', 'face')[kind]} from this mesh")
        return element.key

    def vertex(self, position):
        self.__dict__.pop("_bulk", None)
        k, _ch = self._store.vertex(self._input_point(position))
        return VertexView(self, k)

    def _probe_neighbours(self, k, position):
        p = _xyz(position)
        hit = self._store.probe(tuple(k), p)
        return None if hit is None else VertexView(self, hit)

    def _link_edge(self, v0, v1):
        st = self._store
        a, b = self._own(v0, 0), self._own(v1, 0)
        if a is None or b is None or not st.is_live(a) or not st.is_live(b):
            raise StoreRefusal("_link_edge requires live, representable incidence in this mesh")
        self._dirty(1)
        k, _ch = st.link_edge(a, b)
        return EdgeView(self, k)

    def add_edge(self, a, b):
        self.__dict__.pop("_bulk", None)
        k, _ch = self._store.add_edge(self._input_point(a), self._input_point(b))
        self._dirty(1)
        return EdgeView(self, k)

    def add_face(self, loop_positions, hole_loops=None):
        return store_views.add_face(self, loop_positions, hole_loops)

    def remove_edge(self, e) -> None:
        return store_views.remove_element(self, e, 1)

    def remove_face(self, face) -> None:
        return store_views.remove_element(self, face, 2)

    def relink_edge(self, edge) -> None:
        return store_views.relink_element(self, edge, 1)

    def relink_face(self, face) -> None:
        return store_views.relink_element(self, face, 2)

    def add_hole(self, face, loop_positions):
        return store_views.add_hole(self, face, loop_positions)

    def relink_hole(self, face, ring) -> None:
        self._store.relink_hole(face.key, [v.key for v in ring])
        self.__dict__.pop("_bulk", None)

    def remove_hole(self, face, loop) -> None:
        return store_views.remove_hole(self, face, loop)

    def purge_debris(self):
        """:meth:`Mesh.purge_debris` in the store. A store has no ghost
        entries (design decision 4): the second count is the dropped
        vertices' own cells."""
        n, ghosts, _ch = self._store.purge_debris()
        if n or ghosts:
            self._dirty(1)
        return n, ghosts

    def prune_orphan_vertices(self) -> int:
        return self.purge_debris()[0]

    def drop_orphan_vertices(self, vertices) -> int:
        vertices = list(vertices)
        keys = [k for k in (self._own(v, 0) for v in vertices)
                if k is not None]
        try:
            n, _ch = self._store.drop_orphan_vertices(keys)
        except StoreFallback:
            raise StoreRefusal("drop_orphan_vertices requires live, representable incidence in this mesh")
        if n:
            self._dirty(1)
        return n

    def relink_vertices(self, vertices) -> None:
        vertices = list(vertices)
        keys = [self._own(v, 0) for v in vertices]
        if None in keys:
            raise StoreRefusal("relink_vertices requires live, representable incidence in this mesh")
        self._store.relink_vertices(keys)
        self._dirty(1)

    # -- port step 3: splits and the stitch primitives (store/stitch.rs) --
    # Each is its Mesh method step for step, its dirty bumps counted by the
    # store; attrs (Python dicts) are carried here, as the Python methods
    # carry them. The geometric tests go through QVector3D in Python, so
    # the store needs this Qt's ``length()`` formula
    # (``_native.qt_hypot()``); without one, they run on views here.

    def _live(self, x, kind):
        return store_views.owned(self, x, kind, True)

    def _undo_state(self):
        """What ``revert_stitch`` restores for a record made here: the
        store's snapshot, O(chunks). The per-face attrs dicts are taken as
        they stand at the revert (the record undoes geometry, LIFO, as
        history uses it)."""
        d = _attribute_owner(self)
        return StoreState(self._store.snapshot(), d["_attr_own"],
                          d["_attr_table"], self)

    def fillet_edges(self, keys, radius, segments=8, profile=0.5,
                     measure="radius", corners="round", shape=None):
        """Round live edge keys, returning one StoreChange for the whole edit."""
        from core.fillet import plan_fillet
        plan = plan_fillet(self, [EdgeView(self, k) for k in keys], radius,
                           segments, profile, measure, corners, shape)
        return self._apply_round_plan(plan)

    def bevel_vertices(self, keys, size, segments=8, profile=0.5):
        """Cut live vertex keys, returning one StoreChange for the whole edit."""
        from core.vertex_bevel import plan_vertex_bevel
        plan = plan_vertex_bevel(self, [VertexView(self, k) for k in keys],
                                 size, segments, profile)
        return self._apply_round_plan(plan)

    def _apply_round_plan(self, plan):
        from core.fillet import apply_fillet
        if isinstance(plan, str):
            raise ValueError(plan)
        if plan._store_plan is None:
            raise StoreFallback("native fillet requires a supported Qt vector kernel")
        return apply_fillet(self, plan)

    def split_edge(self, edge, position):
        return store_views.split_edge(self, edge, position)

    def split_edge_at(self, edge, mid) -> dict:
        return store_views.split_edge_at(self, edge, mid)

    def interior_vertex_on(self, edge):
        k, h = self._own(edge, 1), _hypot()
        if k is None:
            return None
        hit = self._store.interior_vertex_on(k, h)
        return None if hit is None else VertexView(self, hit)

    def collapsible_vertex(self, v) -> bool:
        k, h = self._own(v, 0), _hypot()
        if k is None:
            return False
        return self._store.collapsible_vertex(k, h)

    def collapse_vertex(self, v) -> dict:
        return store_views.collapse_vertex(self, v)

    def collapse_collinear(self) -> int:
        """Collapse collinear vertices in one atomic store edit."""
        (n, bumps), _ch = self._store.collapse_collinear(_hypot())
        self._dirty(bumps)
        return n

    def _weld_into(self, keep, dup) -> None:
        a, b = self._live(keep, 0), self._live(dup, 0)
        if a is None or b is None:
            raise StoreRefusal("_weld_into requires live, representable incidence in this mesh")
        try:
            n, _ch = self._store.weld_into(a, b)
        except StoreFallback:
            raise StoreRefusal("_weld_into requires live, representable incidence in this mesh")
        self._dirty(n)

    def weld_coincident(self) -> bool:
        try:
            (changed, n), _ch = self._store.weld_coincident()
        except StoreFallback:
            raise StoreRefusal("coincident weld cannot preserve incidence")
        self._dirty(n)
        return changed

    def finish_push(self, before, face, shift, facet_cos, painted):
        """Publish curve allocation and expose the new faces that inherit paint."""
        _mesh._CURVE_COUNTER, faces = self._store.finish_push(
            before, face, shift, facet_cos, _mesh._CURVE_COUNTER, painted, _hypot())
        return [FaceView(self, key) for key in faces]

    def rebuild_cap_plane(self, origin, normal):
        return store_views.rebuild_cap_plane(self, origin, normal)

    def push_extrude(self, scene, face, distance, normal, keep, oriented, preview):
        return store_views.push_extrude(self, scene, face, distance, normal, keep, oriented, preview)

    def push_translate(self, cap, delta, removing, preview):
        return store_views.push_translate(self, cap, delta, removing, preview)

    def stitch_connect(self, seedkeys, dedupe: bool) -> None:
        return store_views.stitch_connect(self, seedkeys, dedupe)

    def dedupe_faces(self) -> int:
        return store_views.dedupe_faces(self)

    def dissolve_edge(self, edge):
        return store_views.dissolve_edge(self, edge, True)

    def dissolve_edge_recorded(self, edge):
        return store_views.dissolve_edge(self, edge, False)

    def dissolve_coplanar_pair(self, face_a, face_b):
        return store_views.dissolve_coplanar_pair(self, face_a, face_b)

    def dissolve_coplanar_region(self, faces):
        return store_views.dissolve_coplanar_region(self, faces)

    def stitch_arrays(self):
        return store_views.stitch_arrays(self)

    def revert_stitch(self, rec) -> None:
        """Undo a stitch recorded by this store by restoring its snapshot."""
        undo = rec.get("_undo")
        if not isinstance(undo, StoreState) or undo.mesh is not self:
            raise ValueError("stitch record has no snapshot of this mesh")
        self.restore_state(undo)

    def tag_curve(self, loop_points, closed=True):
        self._chunk_dirty = True
        self._mut_serial += 1
        points = list(loop_points)
        cid = _mesh._CURVE_COUNTER
        if not self._store._model_frame:
            matched = _mesh.object_drawing.tag_curve(self.edges, points, closed, cid)
        else:
            matched = self._store.tag_curve_path([p.toTuple() for p in points], closed, cid)
        if matched:
            return self.next_curve_id()
        return None

    def resplit_curves(self) -> None:
        self._chunk_dirty = True
        self._mut_serial += 1
        if self._store._model_frame:
            _mesh._CURVE_COUNTER = self._store.resplit_curves(_mesh._CURVE_COUNTER)
        else:
            _mesh._CURVE_COUNTER = _mesh.object_drawing.resplit_curves(self.edges, _mesh._CURVE_COUNTER)

    def clear(self) -> None:
        self._store.clear()
        attrs = _attribute_owner(self)
        attrs["_attr_own"].clear()
        attrs["_attr_table"] = []
        self._dirty()

    def _journal_state(self, snapshot):
        return store_views.capture_state(self, snapshot)

    def capture_state(self):
        return store_views.capture_state(self)

    def restore_state(self, snap) -> None:
        store_views.restore_state(self, snap)

    # -- bulk construction straight into the store
    def bulk_weld(self, pos):
        return store_views.bulk_weld(self, pos)

    def add_edges_welded(self, vobjs, ia, ib, flags=None):
        return store_views.add_edges_welded(self, vobjs, ia, ib, flags)

    def add_faces_welded(self, vobjs, inverse, ring_sizes, face_ring_counts,
                         attrs=None, edge_map=None):
        return store_views.add_faces_welded(self, vobjs, inverse, ring_sizes,
                                           face_ring_counts, attrs, edge_map)

    def __deepcopy__(self, memo):
        return store_views.deepcopy_mesh(self, memo)

    def __reduce_ex__(self, protocol):
        return store_views.reduce_mesh(self)

    def __setstate__(self, saved):
        store_views.set_mesh_state(self, saved)


def _hypot():
    """Platform rounding convention for native geometric predicates."""
    from core import _native
    return _native.qt_hypot()


def _hole_filter(holes):
    """``Mesh.add_face``'s nested-hole filter (``topology._maximal_holes``)
    over the merged face's holes, given as positions: a bool per hole. The
    store calls it when a dissolve's face has several holes, before it
    changes anything."""
    rings = [[_xyz(p) for p in ring] for ring in holes]
    from core import _native
    return _native.available().MeshStore.maximal_holes(rings, _hypot() or "libstdc++")


def _xyz(p):
    if hasattr(p, "toTuple"):
        return p.toTuple()
    x, y, z = p
    return (float(x), float(y), float(z))


class StoreRefusal(Exception):
    """The requested operation cannot preserve native mesh incidence."""


_STRICT = [0]


class strict:
    """Propagate a native refusal to the enclosing command for rollback."""

    def __enter__(self):
        _STRICT[0] += 1
        return self

    def __exit__(self, *exc):
        _STRICT[0] -= 1
        return False


def _note_refusal(step, err) -> None:
    """Log a native operation refusal and propagate it inside strict commands."""
    import os
    path = os.environ.get("INGETRAZO_REFUSALS")
    if path:
        _log_refusal(path, step, err)
    if _STRICT[0]:
        raise StoreRefusal(f"{step}: {err}" if err is not None else step)


def _log_refusal(path, step, err, **extra) -> None:
    import json
    import os
    import sys
    ops = []
    f = sys._getframe(2)
    while f is not None:
        name = os.path.basename(f.f_code.co_filename)
        if name in ("push_pull.py", "pushpull.py", "cap_rebuild.py", "history.py",
                    "topology.py", "commands.py") and f.f_code.co_name not in ops:
            ops.append(f"{name[:-3]}.{f.f_code.co_name}")
        f = f.f_back
    test = os.environ.get("PYTEST_CURRENT_TEST", "").split(" ")[0]
    with open(path, "a") as out:
        out.write(json.dumps({"step": step, "reason": "" if err is None else str(err)[:200],
                              "ops": ops[:6], "test": test, **extra}) + "\n")


def instance_view(mesh, matrix, origin=(0., 0., 0.)):
    """Reconstruct a placed facade, including from an existing pickle."""
    return store_views.instance_view(mesh, matrix, origin)


def _unpickle_mesh(version, archive, cls=StoreMesh):
    return store_views.unpickle_mesh(version, archive, cls)


class _EdgeMap(Mapping):
    """``add_edges_welded``'s ``{packed vertex pair: edge}`` for a bulk
    store, answered by the store (``find_edge``) instead of a dict of
    half a million views."""

    __slots__ = ("m", "vobjs", "_full")

    def __init__(self, m, vobjs):
        self.m = m
        self.vobjs = vobjs
        self._full = None

    def _pair(self, key):
        u = len(self.vobjs)
        return divmod(key, u)

    def __getitem__(self, key):
        try:
            a, b = self._pair(int(key))
            e = self.m.find_edge(self.vobjs[a], self.vobjs[b])
        except (IndexError, TypeError, ValueError):
            raise KeyError(key) from None
        if e is None:
            raise KeyError(key)
        return e

    def _dict(self):
        if self._full is None:
            u = len(self.vobjs)
            out = {}
            index = {v.key: i for i, v in enumerate(self.vobjs)}
            for e in self.m.edges:
                i0 = index.get(e.v0.key)
                i1 = index.get(e.v1.key)
                if i0 is not None and i1 is not None:
                    out.setdefault(i0 * u + i1 if i0 <= i1 else i1 * u + i0,
                                   e)
            self._full = out
        return self._full

    def __iter__(self):
        return iter(self._dict())

    def __len__(self):
        return len(self._dict())


def faces_arrays(mesh):
    return store_views.mesh_face_arrays(mesh)


def plan_cap_plane(mesh, origin, normal):
    """Read a cap plan in store coordinates."""
    import sys
    plan = mesh._store.single_plane(origin.toTuple(), normal.toTuple(), _hypot(), sys.version_info >= (3, 12))
    if plan is None:
        return None
    def ring(points):
        return [QVector3D(*p) for p in points]
    return [(ring(rings[0]), [ring(r) for r in rings[1:]], part) for rings, part in plan]


def _deferred_state(name):
    def get(self):
        self._materialize()
        return getattr(self, name)

    def put(self, value):
        self._materialize()
        setattr(self, name, value)
    return property(get, put)


class DeferredMesh(StoreMesh):
    """A document block whose native store is built on first geometry access."""

    _store = _deferred_state("_store")
    vertices = _deferred_state("vertices")
    edges = _deferred_state("edges")
    faces = _deferred_state("faces")
    _registry = _deferred_state("_registry")
    _displaced = _deferred_state("_displaced")

    defer = staticmethod(store_views.defer)

    def _materialize(self):
        store_views.materialize(self)

    def __deepcopy__(self, memo):
        self._materialize()
        return self.__deepcopy__(memo)

    def __reduce_ex__(self, protocol):
        # deepcopy / pickle copy the built mesh, never the pending build.
        self._materialize()
        return self.__reduce_ex__(protocol)


def _first_touch(name):
    """Build the deferred store before dispatching the geometry method."""
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
for _name, _value in {**_mesh._MeshState.__dict__, **StoreMesh.__dict__}.items():
    if (callable(_value) and not isinstance(_value, staticmethod)
            and _name not in ("__init__", "__deepcopy__", "bump_revision",
                              "mark_modified", "invalidate_placement_chunks")
            and not (_name.startswith("__") and _name.endswith("__"))):
        setattr(DeferredMesh, _name, _first_touch(_name))
del _name, _value
