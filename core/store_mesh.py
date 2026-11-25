# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Python mesh interfaces backed by the native MeshStore.

Deferred document blocks build on first geometry access. Element views
resolve keys against the active coordinate frame. StoreState shares native
snapshot chunks and preserves per-face Python attributes."""
from __future__ import annotations

from collections.abc import Mapping

import numpy as np
from ingetrazo_core import Vec3 as QVector3D, MeshElements, polygon_centroid, polygon_net_area, polygon_newell as _newell, store_views

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


def _frozen(x):
    """A hashable stand-in for an attrs value: equal values freeze equal."""
    if isinstance(x, dict):
        return ("d", tuple(sorted((k, _frozen(v)) for k, v in x.items())))
    if isinstance(x, list):
        return ("l", tuple(_frozen(v) for v in x))
    if isinstance(x, tuple):
        return ("t", tuple(_frozen(v) for v in x))
    hash(x)
    return x


def paint_classes(mesh, *, push=False):
    """``(own, rows, reps)`` for the native rebuild: the paint class of each
    face's own attrs dict and of each attribute-table row (equal dicts, one
    class; 0 for none) and a dict per class."""
    owner = _attribute_owner(mesh)
    reps = [{}]
    index: dict = {}

    def cls(d):
        if not d:
            return 0
        try:
            key = _frozen(d)
        except TypeError:
            key = None
        if key is not None:
            c = index.get(key)
            if c is None:
                c = index[key] = len(reps)
                reps.append(d)
            return c
        for c, r in enumerate(reps):
            if c and r == d:
                return c
        reps.append(d)
        return len(reps) - 1

    rows = [cls(r) for r in owner["_attr_table"]]
    own = {k: cls(d) for k, d in owner["_attr_own"].items()}
    if push:
        from core.mesh import PAINT_KEYS, _exchange_paint
        # Entry orientation and free-sheet winding can each exchange the sides.
        # The store also needs the paint-only class inherited by a divided cap.
        swaps = []
        for _ in range(2):
            for d in reps[len(swaps):]:
                copy = dict(d)
                if isinstance(copy.get("back"), dict):
                    _exchange_paint(copy)
                swaps.append(cls(copy))
        surface = [cls({k: d[k] for k in PAINT_KEYS if k in d}) for d in list(reps)]
        swaps.extend(range(len(swaps), len(reps)))
        surface.extend(range(len(surface), len(reps)))
        return own, rows, reps, swaps, surface
    return own, rows, reps


def _copy_attrs(d) -> dict:
    """A face's own attrs from a table entry: one level deep, as
    ``formats.igz._face_attrs_from_json`` gives every face its own colour
    list and texture dict."""
    if not d:
        return {}
    return {k: (dict(v) if type(v) is dict else list(v) if type(v) is list
                else v) for k, v in d.items()}


# ---- Views ----------------------------------------------------------------

def _attribute_owner(mesh):
    """The facade holding this definition's face attribute rows."""
    return mesh.__dict__.get("_instance_definition", mesh).__dict__


def _store_of(m):
    """The live store of a store-backed mesh, else None."""
    return m.__dict__.get("_store")


class _ViewMixin:
    __slots__ = ()

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

    def __init__(self, m, key):
        self._m = m
        self.key = key

    @property
    def position(self):
        m = self._m
        return _position(m, self.key)

    def position_tuple(self):
        """Model coordinates for geometry consumers, including sub-float32 edits."""
        st = self._m._store
        return tuple(st.vertex_position(self.key))

    @position.setter
    def position(self, value):
        self._m.place_vertex(self, value)

    @property
    def edges(self):
        m = self._m
        st = m._store
        keys = st.vertex_edges(self.key)
        return _MutSet(EdgeView(m, k) for k in keys)

    @edges.setter
    def edges(self, value):
        if set(value) != set(self.edges):
            raise ValueError("vertex.edges is determined by the endpoints of live edges")


class EdgeView(_ViewMixin, Edge):
    __slots__ = ("_m",)

    def __init__(self, m, key):
        self._m = m
        self.key = key

    @property
    def v0(self):
        m = self._m
        st = m._store
        return VertexView(m, st.edge_ends(self.key)[0])

    @v0.setter
    def v0(self, value):
        self._m._store.set_endpoint(self.key, 0, self._m._incidence_key(value, 0))
        self._m._dirty()

    @property
    def v1(self):
        m = self._m
        st = m._store
        return VertexView(m, st.edge_ends(self.key)[1])

    @v1.setter
    def v1(self, value):
        self._m._store.set_endpoint(self.key, 1, self._m._incidence_key(value, 0))
        self._m._dirty()

    @property
    def faces(self):
        m = self._m
        st = m._store
        keys = st.edge_faces(self.key)
        return _MutList(self, ("faces",), [FaceView(m, k) for k in keys])

    @faces.setter
    def faces(self, value):
        keys = [self._m._incidence_key(f, 2) for f in value]
        self._m._store.order_edge_faces(self.key, keys)
        self._m._dirty()

    def _flag(self, name):
        st = self._m._store
        _a, _b, flags, curve, layer = st.edge(self.key)
        if name == "soft":
            return bool(flags & 1)
        if name == "flat":
            return bool(flags & 4)
        if name == "hidden":
            return bool(flags & 2)
        if name == "curve":
            return None if curve < 0 else curve
        return layer

    def _set_flag(self, name, value):
        m = self._m
        st = m._store
        if name in ("soft", "hidden", "flat"):
            flags = st.edge(self.key)[2]
            bit = {"soft": 1, "hidden": 2, "flat": 4}[name]
            flags = (flags | bit) if value else (flags & ~bit)
            if not isinstance(value, bool) and value not in (0, 1):
                raise TypeError(f"invalid {name} value: {value!r}")
            st.set_edge_flags(self.key, flags)
        elif name == "curve":
            if value is not None and not isinstance(value, int):
                raise TypeError(f"invalid {name} value: {value!r}")
            st.set_edge_curve(self.key, value)
        else:
            if value is not None and not isinstance(value, str):
                raise TypeError(f"invalid {name} value: {value!r}")
            st.set_edge_layer(self.key, value)

    soft = property(lambda self: self._flag("soft"),
                    lambda self, v: self._set_flag("soft", v))
    hidden = property(lambda self: self._flag("hidden"),
                      lambda self, v: self._set_flag("hidden", v))
    flat = property(lambda self: self._flag("flat"),
                    lambda self, v: self._set_flag("flat", v))
    curve = property(lambda self: self._flag("curve"),
                     lambda self, v: self._set_flag("curve", v))
    layer = property(lambda self: self._flag("layer"),
                     lambda self, v: self._set_flag("layer", v))

    @property
    def a(self) -> QVector3D:
        m = self._m
        k = m.__dict__["_store"].edge_ends(self.key)[0]
        return _position(m, k)

    @property
    def b(self) -> QVector3D:
        m = self._m
        k = m.__dict__["_store"].edge_ends(self.key)[1]
        return _position(m, k)

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


def _position(m, key) -> QVector3D:
    """A vertex's position as ONE ``QVector3D`` per vertex until the store
    changes (any op bumps its revision), as a Python vertex hands out its
    own object: scans that memoise by point object
    (``topology.KeyMemo``) hit again, and a planner pass over every edge
    builds each vector once instead of once per read."""
    d = m.__dict__
    st = d["_store"]
    pc = _pos_dict(m, st)
    p = pc.get(key)
    if p is None:
        p = QVector3D(*st.vertex_position(key))
        pc[key] = p
    return p


def _pos_dict(m, st) -> dict:
    """The vertex key -> QVector3D cache of the store's current revision."""
    d = m.__dict__
    rev = st.revision
    cache = d.get("_pos_cache")
    if cache is None or cache[0] != rev or cache[2] is not st:
        cache = d["_pos_cache"] = (rev, {}, st)
    return cache[1]


def _face_entry(m, key) -> list:
    """``[ring positions as tuples, rings as QVector3D lists, normal]`` of a
    face, kept until the store changes: a command scans every face several
    times (``AddFaceCommand``: containment, subdivision, carving read
    ``vertices`` and ``normal()`` ~30 times per face of the ride's largest
    mesh), and each read was a store call plus fresh vectors. Raises
    KeyError for a key the store does not have."""
    d = m.__dict__
    st = d["_store"]
    rev = st.revision
    cache = d.get("_face_cache")
    if cache is None or cache[0] != rev or cache[2] is not st:
        cache = d["_face_cache"] = (rev, {}, st)
    hit = cache[1].get(key)
    if hit is None:
        hit = cache[1][key] = [st.face_positions(key), None, None]
    return hit


class FaceView(_ViewMixin, Face):
    __slots__ = ("_m",)

    def __init__(self, m, key):
        self._m = m
        self.key = key

    def _rings(self):
        m = self._m
        return [[VertexView(m, k) for k in r]
                for r in m._store.face_rings(self.key)]

    @property
    def loop(self):
        m = self._m
        st = m._store
        rings = st.face_rings(self.key)
        return _MutList(self, ("loop",), [VertexView(m, k) for k in rings[0]])

    @loop.setter
    def loop(self, value):
        rings = self._m._store.face_rings(self.key)
        rings[0] = [self._m._incidence_key(v, 0) for v in value]
        self._m._store.replace_rings(self.key, rings)
        self._m._dirty()

    @property
    def hole_loops(self):
        m = self._m
        st = m._store
        rings = st.face_rings(self.key)
        return _MutList(self, ("hole_loops",), [
            _MutList(self, ("hole_loops", i),
                     [VertexView(m, k) for k in r])
            for i, r in enumerate(rings[1:])])

    @hole_loops.setter
    def hole_loops(self, value):
        outer = self._m._store.face_rings(self.key)[0]
        holes = [[self._m._incidence_key(v, 0) for v in ring] for ring in value]
        self._m._store.replace_rings(self.key, [outer, *holes])
        self._m._dirty()

    def _positions(self):
        return _face_entry(self._m, self.key)[0]

    def _vectors(self):
        """The rings as the vertices' own position objects (cached per
        store revision, see ``_face_entry``), or None."""
        m = self._m
        hit = _face_entry(m, self.key)
        if hit[1] is None:
            st = m.__dict__["_store"]
            pc = _pos_dict(m, st)
            rings = []
            for r in st.face_rings(self.key):
                ring = []
                for k in r:
                    p = pc.get(k)
                    if p is None:
                        p = pc[k] = QVector3D(*st.vertex_position(k))
                    ring.append(p)
                rings.append(ring)
            hit[1] = rings
        return hit[1]

    @property
    def vertices(self):
        rings = self._vectors()
        return list(rings[0])

    @property
    def holes(self):
        rings = self._vectors()
        return [list(r) for r in rings[1:]]

    @property
    def interior(self):
        st = self._m._store
        return bool(st.face_info(self.key)[1] & 1)

    @interior.setter
    def interior(self, value):
        st = self._m._store
        if not isinstance(value, bool):
            raise TypeError("face.interior requires a bool")
        flags = st.face_info(self.key)[1]
        st.set_face_flags(self.key, (flags | 1) if value else (flags & ~1))

    @property
    def attrs(self):
        m = self._m
        st = m._store
        attrs = _attribute_owner(m)
        own = attrs["_attr_own"]
        d = own.get(self.key)
        if d is None:
            idx = st.face_info(self.key)[0]
            # The face's own dict from now on, as a Python face has one: a
            # write into it (or into a texture dict inside) stays this
            # face's, and the next read hands back the same object.
            d = own[self.key] = _copy_attrs(
                attrs["_attr_table"][idx]) if idx >= 0 else {}
        return d

    @attrs.setter
    def attrs(self, value):
        m = self._m
        _attribute_owner(m)["_attr_own"][self.key] = value

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

    def normal(self) -> QVector3D:
        m = self._m
        hit = _face_entry(m, self.key)
        n = hit[2]
        if n is None:
            n = store_views.face_normal(hit[0][0])
            hit[2] = n
        return QVector3D(n)          # a caller may scale it in place

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

class StoreState(Mapping):
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

    # -- comparison: capture_state() == state0
    def _face_attrs(self, key, arrays_idx):
        d = self.own.get(key)
        if d is not None:
            return d
        return self.table[arrays_idx] if arrays_idx >= 0 else {}

    def changes_since(self, before):
        """Net key changes including face attributes held outside the native columns."""
        attrs = []
        for k in sorted(self.own.keys() | before.own.keys()):
            if not (self.snap.has(k) and before.snap.has(k)):
                continue
            if self._face_attrs(k, self.snap.face_info(k)[0]) != before._face_attrs(
                    k, before.snap.face_info(k)[0]):
                attrs.append(k)
        return self.snap.changes_since(before.snap, attrs)

    def __eq__(self, other):
        if other is self:
            return True
        if isinstance(other, dict):
            return self._equals_python(other)
        if not isinstance(other, StoreState):
            return NotImplemented
        if not other.snap.same_as(self.snap):
            return False
        if not self.own and not other.own:
            return True
        for k in set(self.own) | set(other.own):
            if not self.snap.has(k):
                continue
            a = self.own.get(k)
            b = other.own.get(k)
            if a is None or b is None:
                idx = self.snap.face_info(k)[0]
                base = self.table[idx] if idx >= 0 else {}
                a = a if a is not None else base
                b = b if b is not None else (
                    other.table[idx] if idx >= 0 else {})
            if a != b:
                return False
        return True

    def __ne__(self, other):
        r = self.__eq__(other)
        return r if r is NotImplemented else not r

    __hash__ = None

    def _equals_python(self, other: dict) -> bool:
        """Compare this snapshot with its plugin-visible mapping representation."""
        for name in self._KEYS:
            if name not in other:
                return False
            mine, theirs = self[name], other[name]
            if name == "vpos":
                if len(mine) != len(theirs) or any(
                        k not in mine or mine[k] != p
                        for k, p in theirs.items()):
                    return False
            elif dict(mine) != theirs if isinstance(mine, Mapping) \
                    else mine != theirs:
                return False
        return True

    # -- the Python snapshot's entries
    _KEYS = ("vertices", "edges", "faces", "registry", "vpos", "vedges",
             "efaces", "esoft", "eflat", "ehidden", "elayer", "ecurve", "floop",
             "fholes", "fattrs", "finterior")

    def __iter__(self):
        return iter(self._KEYS)

    def __len__(self):
        return len(self._KEYS)

    def __getitem__(self, name):
        hit = self._cache.get(name)
        if hit is not None:
            return hit
        if name not in self._KEYS:
            raise KeyError(name)
        hit = (_SnapPositions(self.snap) if name == "vpos" else
               self.snap.field(name, self.mesh, self.own, self.table))
        self._cache[name] = hit
        return hit


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
        initialise_store(self)

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
        table, fa = attr_columns(self)
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
        d = self.__dict__
        d.pop("_bulk", None)
        if n:
            d["_chunk_dirty_value"] = True
            d["_revision"] = d.get("_revision", 0) + n
            d["_mut_serial"] = d.get("_mut_serial", 0) + n

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
        """``x``'s key when it names an element of this store of ``kind``
        (0 vertex, 1 edge, 2 face), listed or removed; else None."""
        k = getattr(x, "key", None)
        if type(k) is not int or k < 0 or (k >> 38) & 3 != kind:
            return None
        return k if self._store.knows(k) else None

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
        self._dirty(1)
        loop = list(loop_positions)
        raw = [list(h) for h in (hole_loops or [])]
        if len(raw) > 1:
            try:
                mask = _hole_filter(raw)
                raw = [ring for ring, keep in zip(raw, mask) if keep]
            except Exception:
                # Python welds the outer loop before filtering the holes.
                for p in loop:
                    self.vertex(p)
                raise
        rings = [[self._input_point(p) for p in loop]] + [[self._input_point(p) for p in h]
                                             for h in raw]
        k, _ch = self._store.add_face(rings)
        self._dirty(sum(len(r) for r in rings))
        return FaceView(self, k)

    def remove_edge(self, e) -> None:
        k = self._own(e, 1)
        if k is None:
            raise StoreRefusal("remove_edge requires live, representable incidence in this mesh")
        self._dirty(1)
        self._store.remove_edge(k)

    def remove_face(self, face) -> None:
        k = self._own(face, 2)
        if k is None:
            raise StoreRefusal("remove_face requires live, representable incidence in this mesh")
        try:
            self._store.remove_face(k)
        except _fallback():
            raise StoreRefusal("remove_face requires live, representable incidence in this mesh")
        self._dirty(1)

    def relink_edge(self, edge) -> None:
        k = self._own(edge, 1)
        if k is None:
            raise StoreRefusal("relink_edge requires live, representable incidence in this mesh")
        try:
            done, _ch = self._store.relink_edge(k)
        except _fallback():
            raise StoreRefusal("relink_edge requires live, representable incidence in this mesh")
        self.__dict__.pop("_bulk", None)
        if done:
            self.mark_modified()

    def relink_face(self, face) -> None:
        k = self._own(face, 2)
        if k is None:
            raise StoreRefusal("relink_face requires live, representable incidence in this mesh")
        st = self._store
        try:
            done, _ch = st.relink_face(k)
        except _fallback():
            raise StoreRefusal("relink_face requires live, representable incidence in this mesh")
        self.__dict__.pop("_bulk", None)
        if done:
            self._dirty(sum(len(r) for r in st.face_rings(k)))
            self.mark_modified()

    def add_hole(self, face, loop_positions):
        k = self._own(face, 2)
        loop = list(loop_positions)
        if k is None or not self._store.is_live(k):
            raise StoreRefusal("add_hole requires live, representable incidence in this mesh")
        try:
            keys, _ch = self._store.add_hole(k, [self._input_point(p) for p in loop])
        except _fallback():
            raise StoreRefusal("add_hole requires live, representable incidence in this mesh")
        self._dirty(len(loop))
        self.__dict__.pop("_bulk", None)
        return [VertexView(self, v) for v in keys]

    def relink_hole(self, face, ring) -> None:
        self._store.relink_hole(face.key, [v.key for v in ring])
        self.__dict__.pop("_bulk", None)

    def remove_hole(self, face, loop) -> None:
        k = self._own(face, 2)
        loop = list(loop)
        vk = [self._own(v, 0) for v in loop]
        if k is None or None in vk:
            raise StoreRefusal("remove_hole requires live, representable incidence in this mesh")
        self.__dict__.pop("_bulk", None)
        self._store.remove_hole(k, vk)          # Python bumps nothing

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
        except _fallback():
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
        """``x``'s key when it names a LIVE element of this store."""
        k = self._own(x, kind)
        return k if k is not None and self._store.is_live(k) else None

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
            raise _fallback()("native fillet requires a supported Qt vector kernel")
        return apply_fillet(self, plan)

    def split_edge(self, edge, position):
        k = self._live(edge, 1)
        if k is None:
            raise StoreRefusal("split_edge requires live, representable incidence in this mesh")
        try:
            (e0, e1, n), _ch = self._store.split_edge(k, self._input_point(position))
        except _fallback():
            raise StoreRefusal("split_edge requires live, representable incidence in this mesh")
        self._dirty(n)
        return EdgeView(self, e0), EdgeView(self, e1)

    def split_edge_at(self, edge, mid) -> dict:
        k, m = self._live(edge, 1), self._live(mid, 0)
        if k is None or m is None:
            raise StoreRefusal("split_edge_at requires live, representable incidence in this mesh")
        undo = self._undo_state()
        try:
            (e0, e1, n0, n1, faces), _ch = self._store.split_edge_at(k, m)
        except _fallback():
            raise StoreRefusal("split_edge_at requires live, representable incidence in this mesh")
        self._dirty(3)
        # "edge" is the split edge's key, which a new first half took.
        return {"kind": "split", "edge": EdgeView(self, k),
                "e0": EdgeView(self, e0), "e1": EdgeView(self, e1),
                "e0_new": n0, "e1_new": n1, "mid": VertexView(self, m),
                "faces": [FaceView(self, f) for f in faces], "_undo": undo}

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
        k = self._live(v, 0)
        if k is None:
            raise StoreRefusal("collapse_vertex requires live, representable incidence in this mesh")
        undo = self._undo_state()
        try:
            (e1, e2, n1, n2, ne, new, faces), _ch = \
                self._store.collapse_vertex(k)
        except _fallback():
            raise StoreRefusal("collapse_vertex requires live, representable incidence in this mesh")
        self._dirty(3)
        return {"kind": "collapse", "v": VertexView(self, k),
                "e1": EdgeView(self, e1), "e2": EdgeView(self, e2),
                "n1": VertexView(self, n1), "n2": VertexView(self, n2),
                "new_edge": EdgeView(self, ne), "new_edge_new": new,
                "faces": [FaceView(self, f) for f in faces], "_undo": undo}

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
        except _fallback():
            raise StoreRefusal("_weld_into requires live, representable incidence in this mesh")
        self._dirty(n)

    def weld_coincident(self) -> bool:
        try:
            (changed, n), _ch = self._store.weld_coincident()
        except _fallback():
            raise StoreRefusal("coincident weld cannot preserve incidence")
        self._dirty(n)
        return changed

    def finish_push(self, before, face, shift, facet_cos, painted):
        """Publish curve allocation and expose the new faces that inherit paint."""
        _mesh._CURVE_COUNTER, faces = self._store.finish_push(
            before, face, shift, facet_cos, _mesh._CURVE_COUNTER, painted, _hypot())
        return [FaceView(self, key) for key in faces]

    def rebuild_cap_plane(self, origin, normal):
        """Publish the native single-plane edit and the donors of its replacement caps."""
        import sys
        own, rows, reps = paint_classes(self)
        bumps, painted = self._store.rebuild_single_plane(
            origin.toTuple(), normal.toTuple(), own, rows, _hypot(), sys.version_info >= (3, 12))
        if bumps:
            self._dirty(bumps)
            for key, cls in painted:
                FaceView(self, key).attrs = _copy_attrs(reps[cls])
        return bool(bumps)

    def push_extrude(self, scene, face, distance, normal, keep, oriented, preview):
        """Publish one store extrusion's paint, selection and editor counters."""
        import sys
        from core.mesh import swap_orientation_paint
        own, rows, reps, swapped, surface = paint_classes(self, push=True)
        try:
            dirty, steps, painted, flips, faces, edges = self._store.push_extrude(
                face.key, distance, normal, keep, oriented, preview, own, rows,
                swapped, surface, _hypot(), sys.version_info >= (3, 12))
        except _fallback():
            _note_refusal("push_extrude")
            raise
        self._dirty(dirty)
        for key, cls in painted:
            FaceView(self, key).attrs = _copy_attrs(reps[cls])
        for key in flips:
            swap_orientation_paint(FaceView(self, key))
        for key in faces:
            scene.selection.discard(FaceView(self, key))
        for key in edges:
            scene.selection.discard(EdgeView(self, key))
        scene.version += steps

    def push_translate(self, cap, delta, removing, preview):
        """Run the prism translation and publish the store's paint transfers."""
        import sys
        from core.mesh import swap_orientation_paint
        own, rows, reps = paint_classes(self)
        try:
            bumps, next_id, before, painted, after, replaced, flipped = self._store.push_translate(
                [p.toTuple() for p in cap], delta, removing, preview, _mesh._CURVE_COUNTER,
                own, rows, _hypot(), sys.version_info >= (3, 12))
        except _fallback():
            _note_refusal("push_translate")
            raise
        _mesh._CURVE_COUNTER = next_id
        self._dirty(bumps)
        for old, new in before:
            FaceView(self, new).attrs = dict(FaceView(self, old).attrs)
        for key, cls in painted:
            FaceView(self, key).attrs = _copy_attrs(reps[cls])
        for old, new in after:
            drop, keep = FaceView(self, old), FaceView(self, new)
            if drop.attrs and not keep.attrs:
                keep.attrs = dict(drop.attrs)
        for old, new in replaced:
            FaceView(self, new).attrs = dict(FaceView(self, old).attrs)
        for key in flipped:
            swap_orientation_paint(FaceView(self, key))

    def stitch_connect(self, seedkeys, dedupe: bool) -> None:
        """Weld, deduplicate and stitch the seeded geometry atomically."""
        from core.mesh import _STITCH_TOL
        (pairs, n), _ch = self._store.stitch_connect(
            sorted(seedkeys or ()), dedupe, _STITCH_TOL, _hypot())
        self._dirty(n)
        for d, k in pairs:
            drop, keep = FaceView(self, d), FaceView(self, k)
            if drop.attrs and not keep.attrs:
                keep.attrs = dict(drop.attrs)

    def dedupe_faces(self) -> int:
        (pairs, n), _ch = self._store.dedupe_faces()
        self._dirty(n)
        for d, k in pairs:
            drop, keep = FaceView(self, d), FaceView(self, k)
            if drop.attrs and not keep.attrs:
                keep.attrs = dict(drop.attrs)
        return len(pairs)

    def _dissolve_edge(self, name, edge, tolerant):
        k, h = self._live(edge, 1), _hypot()
        if k is None:
            raise StoreRefusal(f"{name} requires representable edge incidence")
        undo = None if tolerant else self._undo_state()
        try:
            r, _ch = self._store.dissolve_edge(k, h, tolerant, _hole_filter)
        except _fallback():
            raise StoreRefusal(f"{name} requires representable edge incidence")
        if r is None:
            return None
        merged, (fa, fb), n = r
        self._dirty(n)
        a, b = FaceView(self, fa), FaceView(self, fb)
        new = FaceView(self, merged)
        new.attrs = dict(_mesh._dominant_attrs((a, b)))
        if tolerant:
            return new
        return {"kind": "merge", "edge": EdgeView(self, k), "face_a": a,
                "face_b": b, "merged": new, "_undo": undo}

    def dissolve_edge(self, edge):
        return self._dissolve_edge("dissolve_edge", edge, True)

    def dissolve_edge_recorded(self, edge):
        return self._dissolve_edge("dissolve_edge_recorded", edge, False)

    def dissolve_coplanar_pair(self, face_a, face_b):
        ka, kb, h = self._live(face_a, 2), self._live(face_b, 2), _hypot()
        if ka is None or kb is None:
            raise StoreRefusal("dissolve_coplanar_pair requires live, representable incidence in this mesh")
        undo = self._undo_state()
        try:
            r, _ch = self._store.dissolve_pair(ka, kb, h, _hole_filter)
        except _fallback():
            raise StoreRefusal("dissolve_coplanar_pair requires live, representable incidence in this mesh")
        if r is None:
            return None
        merged, edges, n = r
        self._dirty(n)
        a, b = FaceView(self, ka), FaceView(self, kb)
        new = FaceView(self, merged)
        new.attrs = dict(_mesh._dominant_attrs((a, b)))
        return {"kind": "merge_pair", "face_a": a, "face_b": b,
                "edges": [EdgeView(self, e) for e in edges], "merged": new,
                "_undo": undo}

    def dissolve_coplanar_region(self, faces):
        """:meth:`Mesh.dissolve_coplanar_region`, the faces taken in the
        caller's order (it decides the reference normal, the dominant
        attrs on ties and where the merged face's loop starts)."""
        faces = list(faces)
        if len(faces) < 2:
            return None
        keys, h = [self._live(f, 2) for f in faces], _hypot()
        if None in keys:
            raise StoreRefusal("dissolve_coplanar_region requires live, representable incidence in this mesh")
        try:
            r, _ch = self._store.dissolve_region(keys, h, _hole_filter)
        except _fallback():
            raise StoreRefusal("dissolve_coplanar_region requires live, representable incidence in this mesh")
        if r is None:
            return None
        stacked, fk, n = r
        self._dirty(n)
        views = [FaceView(self, k) for k in keys]
        if stacked:
            first = views[0]
            if not first.attrs:
                first.attrs = dict(_mesh._dominant_attrs(views[1:]))
            return first
        new = FaceView(self, fk)
        new.attrs = dict(_mesh._dominant_attrs(views))
        return new

    def stitch_arrays(self):
        """``(positions (V, 3) float64, edge ends (E, 2) int64 as list
        indices, vertices, edges)`` for ``topology.sweep_tjunctions``: the
        store's arrays, and the elements as sequences that make a view
        only for the index asked (the sweep splits a handful of edges out
        of hundreds of thousands)."""
        st = self._store
        a = st.arrays()
        return (np.asarray(a["pos"], dtype=np.float64),
                np.asarray(a["edges"], dtype=np.int64),
                _Indexed(self, st.keys("vertex"), VertexView),
                _Indexed(self, st.keys("edge"), EdgeView))

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
        """Attach Python face paint to a canonical document store snapshot."""
        from copy import deepcopy
        d = _attribute_owner(self)
        own = {k: deepcopy(v) if v else {} for k, v in d["_attr_own"].items()}
        return StoreState(snapshot, own, d["_attr_table"], self)

    def capture_state(self):
        d = _attribute_owner(self)
        return StoreState(self._store.snapshot(),
                          {k: dict(v) for k, v in d["_attr_own"].items()},
                          d["_attr_table"], self)

    def restore_state(self, snap) -> None:
        if isinstance(snap, StoreState):
            _restore_store_state(self, snap)
            return
        raise ValueError("restore_state requires a native mesh snapshot")

    # -- bulk construction straight into the store
    def bulk_weld(self, pos):
        if self._store.revision == 0 and not any(self._store.counts()) and self._store._model_frame:
            return bulk_weld(self, pos)
        points = np.ascontiguousarray(pos, dtype=np.float64)
        if not len(points):
            return [], np.empty(0, dtype=np.int64)
        planned = self.__dict__.get("_planned_points")
        supplied = (np.asarray([planned.get(tuple(p), p) for p in points], dtype=np.float64)
                    if planned else None)
        keys, indices = self._store.weld_existing(points, supplied)
        self.__dict__.pop("_bulk", None)
        return [VertexView(self, k) for k in keys], indices

    def add_edges_welded(self, vobjs, ia, ib, flags=None):
        b = self.__dict__.get("_bulk")
        if b is None or vobjs is not b["vobjs"] or b["stage"] != "points":
            pairs = [(int(a), int(ib[i])) for i, a in enumerate(ia)]
            # Python appends a parallel edge if one already existed before
            # this batch. The store represents one edge per unordered pair.
            if any(self._live(v, 0) is None for v in vobjs) or any(
                    self.find_edge(vobjs[a], vobjs[b]) is not None
                    for a, b in pairs if vobjs[a].key != vobjs[b].key):
                raise StoreRefusal("add_edges_welded requires live, representable incidence in this mesh")
            if flags is not None and any(
                    (row[1] is not None and not isinstance(row[1], int))
                    or (row[2] is not None and not isinstance(row[2], str))
                    for row in flags):
                raise StoreRefusal("add_edges_welded requires live, representable incidence in this mesh")
            self._bump()
            emap = {}
            for index, (a, b) in enumerate(pairs):
                if vobjs[a].key == vobjs[b].key:
                    continue
                packed = min(a, b) * len(vobjs) + max(a, b)
                if packed not in emap:
                    key, _ = self._store.link_edge(vobjs[a].key, vobjs[b].key)
                    emap[packed] = EdgeView(self, key)
                if flags is not None:
                    for name, value in zip(_mesh.EDGE_FLAG_NAMES, flags[index]):
                        if value is not None and (name in ("layer", "curve") or value):
                            setattr(emap[packed], name, value if name in ("layer", "curve") else True)
            return emap
        return _bulk_edges(self, b, ia, ib, flags)

    def add_faces_welded(self, vobjs, inverse, ring_sizes, face_ring_counts,
                         attrs=None, edge_map=None):
        b = self.__dict__.get("_bulk")
        if (b is None or vobjs is not b["vobjs"]
                or b["stage"] not in ("points", "edges")
                or not (edge_map is None or isinstance(edge_map, _EdgeMap))):
            if (any(n < 1 for n in face_ring_counts) or any(n < 1 for n in ring_sizes)
                    or any(self._live(v, 0) is None for v in vobjs)) or (
                    edge_map is not None and not isinstance(edge_map, (dict, _EdgeMap))):
                raise StoreRefusal("bulk faces require vertices and edges from this store")
            ring_ends = np.cumsum(ring_sizes)
            rings = [inverse[end - size:end] for end, size in zip(ring_ends, ring_sizes)]
            face_ends = np.cumsum(face_ring_counts)
            polygons = [rings[end - count:end] for end, count in zip(face_ends, face_ring_counts)]
            # A caller-supplied map may deliberately request a parallel
            # edge or a different edge's incidence. Only objects model that.
            pairs = {}
            if isinstance(edge_map, dict):
                for rings in polygons:
                    for ring in rings:
                        for index, a in enumerate(ring):
                            b = ring[(index + 1) % len(ring)]
                            pairs[min(a, b) * len(vobjs) + max(a, b)] = (a, b)
                if any(edge_map.get(k) != self.find_edge(vobjs[a], vobjs[b])
                       for k, (a, b) in pairs.items()):
                    raise StoreRefusal("bulk faces require vertices and edges from this store")
            self._bump()
            faces = []
            for i, rings in enumerate(polygons):
                keys = [[vobjs[j].key for j in ring] for ring in rings]
                key, _ = self._store.add_welded_face(keys)
                face = FaceView(self, key)
                if attrs is not None and attrs[i]:
                    face.attrs.update(attrs[i])
                faces.append(face)
            for k, (a, b) in pairs.items():
                edge_map[k] = self.find_edge(vobjs[a], vobjs[b])
            return faces
        return _bulk_faces(self, b, inverse, ring_sizes, face_ring_counts,
                           attrs)

    # -- copies
    def __deepcopy__(self, memo):
        from copy import deepcopy
        definition = self.__dict__.get("_instance_definition")
        if definition is not None:
            store = self._store
            new = instance_view(deepcopy(definition, memo), store.frame_matrix, store.frame_origin)
            memo[id(self)] = new
            return new
        new = object.__new__(type(self))
        memo[id(self)] = new
        d = self.__dict__
        rest = {k: v for k, v in d.items()
                if k not in STORE_SIDE and k != "_keys"
                and k not in ("vertices", "edges", "faces", "_registry",
                              "_displaced")}
        new.__dict__.update(deepcopy(rest, memo))
        ns = next(_mesh._KEY_NAMESPACES)
        new.__dict__["_store"] = d["_store"].copy(ns)
        new.__dict__["_attr_table"] = d["_attr_table"]
        new.__dict__["_attr_own"] = {
            (ns << 40) | (k & LOCAL_MASK): deepcopy(v, memo)
            for k, v in d["_attr_own"].items()}
        return new

    def __reduce_ex__(self, protocol):
        definition = self.__dict__.get("_instance_definition")
        if definition is not None:
            return instance_view, (definition, self._store.frame_matrix, self._store.frame_origin)
        state = {k: v for k, v in self.__dict__.items() if k not in STORE_SIDE and k != "_keys"}
        attrs = {k & LOCAL_MASK: v for k, v in self._attr_own.items()}
        return _unpickle_mesh, (1, self._store.pickle_archive(), type(self)), (state, self._attr_table, attrs)

    def __setstate__(self, saved):
        state, table, attrs = saved
        self.__dict__.update(state)
        self._attr_table = table
        self._attr_own = {self._store.key_base | k: v for k, v in attrs.items()}

def _fallback():
    """``ingetrazo_core.StoreFallback``: an op the store refused before
    changing anything (store/topo.rs); the Python kernel runs instead.
    From the loaded module itself: a store exists, so it is loaded, even
    when ``_native.native`` was fixed to None for the session."""
    from core import _native
    return _native.available().StoreFallback


class _Indexed:
    """A list of views made on demand: ``len`` and ``[i]``."""

    __slots__ = ("_m", "_keys", "_cls")

    def __init__(self, m, keys, cls):
        self._m, self._keys, self._cls = m, keys, cls

    def __len__(self):
        return len(self._keys)

    def __getitem__(self, i):
        return self._cls(self._m, self._keys[i])


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


def _note_refusal(step) -> None:
    """Log a native operation refusal and propagate it inside strict commands."""
    import os
    import sys
    err = sys.exc_info()[1]
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


def _restore_store_state(m, snap: StoreState) -> None:
    """``m.restore_state(snap)`` for a store snapshot: ``m`` is store-backed
    afterwards. Same lineage: the store goes back to it, keys kept. Another
    store's snapshot is adopted under a fresh
    namespace: new keys (design decision 3)."""
    from copy import deepcopy
    d = m.__dict__
    st = snap.owner if snap.mesh is m else d.get("_store")
    current = d.get("_store")
    if snap.mesh is m and current is not None and current.lineage == st.lineage:
        st = current
    d["_store"] = st
    foreign = st is None or st.lineage != snap.snap.lineage
    if st is None:
        from core import _native
        st = d["_store"] = _native.available().MeshStore(
            next(_mesh._KEY_NAMESPACES), **_weld_args())
    if foreign:
        ns = next(_mesh._KEY_NAMESPACES)
        st.restore(snap.snap, namespace=ns)
        own = {(ns << 40) | (k & LOCAL_MASK): deepcopy(v) if v else {}
               for k, v in snap.own.items()}
    else:
        held = _attribute_owner(m).get("_attr_own") or {}
        st.restore(snap.snap)
        own = {k: deepcopy(v) if v else {} for k, v in snap.own.items()}
        # The restore kept the newer elements as tombstones (restore_tail
        # .rs); their own attrs come along for a relink.
        for k, v in held.items():
            if k not in own and not snap.snap.has(k) and st.knows(k):
                own[k] = v
    d["_attr_table"] = snap.table
    d["_attr_own"] = own
    definition = d.get("_instance_definition")
    if definition is not None:
        definition._attr_own = own
        definition._attr_table = snap.table
    for name in ("vertices", "edges", "faces", "_registry", "_displaced",
                 "_bulk"):
        d.pop(name, None)
    if type(m) is DeferredMesh:
        m.__class__ = StoreMesh
    m._chunk_dirty = True
    m._mut_serial += 1


def instance_view(mesh, matrix, origin=(0., 0., 0.)):
    """An occurrence facade over the definition store, with the same element keys."""
    if isinstance(mesh, DeferredMesh):
        mesh._materialize()
    if type(mesh) is not StoreMesh:
        return None
    try:
        view = mesh._store._instance_view(mesh, matrix, origin)
    except ValueError:  # a singular placement has no world-to-definition inverse
        return None
    result = StoreMesh()
    become_store(result, view, mesh._attr_table, mesh._attr_own)
    result._instance_definition = mesh
    return result


def _unpickle_mesh(version, archive, cls=StoreMesh):
    from ingetrazo_core import MeshStore
    mesh = object.__new__(cls)
    store = MeshStore.from_pickle_archive(version, archive, next(_mesh._KEY_NAMESPACES))
    become_store(mesh, store, [])
    return mesh


def initialise_store(mesh):
    from core import _native
    store = _native.available().MeshStore(next(_mesh._KEY_NAMESPACES), **_weld_args())
    become_store(mesh, store, [])


def become_store(m, store, table, own=None) -> None:
    """Make the plain, empty ``m`` store-backed by ``store``."""
    d = m.__dict__
    for name in ("vertices", "edges", "faces", "_registry", "_displaced"):
        d.pop(name, None)
    d["_store"] = store
    d["_attr_table"] = table
    d["_attr_own"] = own if own is not None else {}
    if type(m) is DeferredMesh:
        m.__class__ = StoreMesh


def _table_index(table, index, a):
    """The attribute-table row for the dict ``a`` (interned by content)."""
    if not a:
        return -1
    key = repr(a)
    i = index.get(key)
    if i is None:
        i = index[key] = len(table)
        table.append(_copy_attrs(a))
    return i


def _input_positions(pos):
    points = np.asarray(pos)
    dtype = np.float32 if points.dtype == np.float32 else np.float64
    return np.ascontiguousarray(points, dtype=dtype).reshape(-1, 3)


def bulk_weld(m, pos):
    """``Mesh.bulk_weld`` on an empty mesh, into a new store. Returns the
    views of the welded vertices and each point's vertex index, as the
    Python method does."""
    from core import _native
    mod = _native.available()
    points = _input_positions(pos)
    if not len(points):
        return [], np.empty(0, dtype=np.int64)
    store, indices = mod.MeshStore._bulk_points(next(_mesh._KEY_NAMESPACES), points, **_weld_args())
    m._store._extend_bulk(store)
    store = m._store
    vobjs = [VertexView(m, k) for k in store.keys("vertex")]
    m.__dict__["_bulk"] = dict(stage="points", vobjs=vobjs)
    return vobjs, indices


def _bulk_edges(m, b, ia, ib, flags):
    ia = np.ascontiguousarray(ia, dtype=np.int64).reshape(-1)
    ib = np.ascontiguousarray(ib, dtype=np.int64).reshape(-1)
    if not m._store._bulk_edges(ia, ib, flags):
        raise StoreRefusal("unsupported bulk edge flags or incidence")
    m._chunk_dirty = True
    m._mut_serial += 1
    b["stage"] = "edges"
    return _EdgeMap(m, b["vobjs"])


def _bulk_faces(m, b, inverse, ring_sizes, face_ring_counts, attrs):
    inverse = np.ascontiguousarray(inverse, dtype=np.int64).reshape(-1)
    sizes = np.ascontiguousarray(ring_sizes, dtype=np.int64).reshape(-1)
    counts_ = np.ascontiguousarray(face_ring_counts,
                                   dtype=np.int64).reshape(-1)
    if not len(counts_):
        m._chunk_dirty = True
        m._mut_serial += 1
        return []
    if (sizes < 1).any() or (counts_ < 1).any():
        raise ValueError("bulk face ring sizes and counts must be positive")
    st = m.__dict__["_store"]
    table = m.__dict__["_attr_table"]
    fa = None
    if attrs is not None:
        index: dict = {}
        by_id: dict = {}
        fa = np.full(len(counts_), -1, dtype=np.int64)
        for i, a in enumerate(attrs):
            if not a:
                continue
            j = by_id.get(id(a))
            if j is None:
                j = by_id[id(a)] = _table_index(table, index, a)
            fa[i] = j
    m._chunk_dirty = True
    m._mut_serial += 1
    st._bulk_faces(inverse, sizes, counts_, fa)
    m.__dict__.pop("_bulk", None)
    return [FaceView(m, k) for k in st.keys("face")]


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


def store_from_source(src, hooks=(), *, precise=False):
    """The store a deferred mesh's ``_deferred_source`` builds, with its
    attribute table. ``hooks`` are the after-build functions links attached
    (``core.links._retable_deferred``): per-element relabels of attrs and
    edge tags, applied here to the table rows and tag names once instead of
    to every face — the same result, since every face of one row has the
    same attrs."""
    from formats.igz import _face_attrs_from_json
    import ingetrazo_core as mod
    meta = src.get("meta") or {}
    raw = meta.get("face_attrs") or []
    table = [_face_attrs_from_json(row) or {} for row in raw]
    names = list(meta.get("edge_layers") or [])
    n_e = len(src["edges"]) if src.get("edges") is not None else 0
    from ingetrazo_core import document_native_io as native_io
    direct = native_io() is not None
    def indices(data):
        a = np.asarray(data)
        dtype = a.dtype if direct and a.dtype in (np.int32, np.int64) else np.int64
        return np.ascontiguousarray(a, dtype=dtype)
    fa = src.get("face_attr")
    nfaces = len(src["counts"])
    fa = (np.full(nfaces, -1, dtype=np.int64) if fa is None
          else indices(fa))
    layers = src.get("layers")
    layers = None if layers is None else indices(layers)
    if hooks:
        table, fa, names, layers = _apply_hooks(hooks, table, fa, names,
                                                layers, n_e)
    w = _weld_args()
    ns = next(_mesh._KEY_NAMESPACES)
    pos = (np.ascontiguousarray(src["pos"], dtype=np.float64).reshape(-1, 3)
           if precise else _input_positions(src["pos"]))
    edges = indices(src["edges"]).reshape(-1, 2)
    flags = src.get("flags")
    curves = src.get("curves")
    store = mod.MeshStore.from_block(
        ns, pos, edges,
        None if flags is None else np.ascontiguousarray(flags, np.uint8),
        None if curves is None else indices(curves),
        layers,
        names,
        indices(src["rings"]),
        indices(src["sizes"]),
        indices(src["counts"]),
        fa,
        sequential=bool(src.get("sequential", False)), **w)
    return store, table


class _RowFace:
    __slots__ = ("attrs",)

    def __init__(self, attrs):
        self.attrs = attrs


class _RowEdge:
    __slots__ = ("layer",)

    def __init__(self, layer):
        self.layer = layer


def _apply_hooks(hooks, table, fa, names, layers, n_e):
    """Run per-element after-build hooks on one stand-in per table row and
    per tag name (plus one for the untagged / attribute-less elements)."""
    from types import SimpleNamespace
    rows = [_RowFace(dict(t)) for t in table] + [_RowFace({})]
    tags = [_RowEdge(n) for n in names] + [_RowEdge(None)]
    stand_in = SimpleNamespace(faces=rows, edges=tags)
    for hook in hooks:
        hook(stand_in)
    table = [r.attrs for r in rows]
    fa = np.where(fa < 0, len(table) - 1, fa)
    new_names: list = []
    ix: dict = {}
    remap = []
    for t in tags:
        if t.layer is None:
            remap.append(-1)
        else:
            j = ix.get(t.layer)
            if j is None:
                j = ix[t.layer] = len(new_names)
                new_names.append(t.layer)
            remap.append(j)
    remap = np.asarray(remap, dtype=np.int64)
    if layers is None:
        layers = np.full(n_e, -1, dtype=np.int64)
    layers = remap[np.where(layers < 0, len(tags) - 1, layers)]
    return table, fa, new_names, layers


#: The arrays a deferred source must carry to be built as a store.
_BLOCK = ("pos", "edges", "rings", "sizes", "counts")


def from_deferred(m) -> bool:
    """A ``DeferredMesh`` on first touch: store-backed instead of built.
    False when the store is off (the caller builds Python objects)."""
    d = m.__dict__
    src = d.get("_deferred_source")
    if src is None or any(src.get(k) is None for k in _BLOCK):
        # Not a loader's block (a hand-made deferral): its own build.
        return False
    hooks = d.pop("_deferred_hooks", ())
    if hooks:
        # The build the hooks follow reads the loader's own tables, not the
        # relabelled ones the drawing shows (core.links._retable_deferred).
        src = d.get("_deferred_hook_source", src)
    store, table = store_from_source(src, hooks)
    for key in ("_deferred", "_deferred_nfaces", "_deferred_source",
                "_deferred_hook_source", "_deferred_geom",
                "_deferred_plan", "_drawing_staged", "_drawing_source_key",
                "_drawing_texture_paths"):
        d.pop(key, None)
    become_store(m, store, table)
    return True


def face_attrs(m):
    """``core.mesh.face_attrs`` of a store-backed mesh: the first face of
    each distinct attribute row, and every face that has its own dict —
    without giving the others one."""
    st = m.__dict__["_store"]
    attrs = _attribute_owner(m)
    own = attrs["_attr_own"]
    table = attrs["_attr_table"]
    fa = st.arrays()["face_attr"]
    keys = st.keys("face") if own else None
    seen = set()
    for i, k in enumerate(fa.tolist()):
        if own:
            d = own.get(keys[i])
            if d is not None:
                yield d
                continue
        if k in seen:
            continue
        seen.add(k)
        yield _copy_attrs(table[k]) if k >= 0 else {}


def attr_columns(m, overrides=None, *, face_attr=None):
    """``(table, face_attr)`` of a store-backed mesh for array readers
    (pack, takeoff): every face's attrs as an index into ``table`` (-1:
    none). Faces with their own dict (touched through a view) or an entry
    in ``overrides`` (face key → attrs) get a row of their own; the others
    share the attribute table's rows."""
    d = m.__dict__
    st = d["_store"]
    fa = st.arrays()["face_attr"].copy() if face_attr is None else face_attr.copy()
    attrs = _attribute_owner(m)
    table = list(attrs["_attr_table"])
    own = attrs["_attr_own"]
    if own or overrides:
        keys = st.keys("face")
        for i, k in enumerate(keys):
            a = (overrides or {}).get(k)
            if a is None:
                a = own.get(k)
            if a is not None:
                fa[i] = len(table)
                table.append(a)
    return table, fa


def faces_arrays(m):
    """``core.geometry_arrays.faces_arrays(mesh.faces)`` of a store-backed
    mesh from its arrays: vertices in first-use order over the face
    corners, as the object walk numbers them."""
    from core.geometry_arrays import Arrays
    a = m.__dict__["_store"].arrays()
    corners = a["corners"]
    if len(corners):
        uniq, first = np.unique(corners, return_index=True)
        order = uniq[np.argsort(first, kind="stable")]
        rank = np.empty(len(a["pos"]), dtype=np.int64)
        rank[order] = np.arange(len(order))
        pos = a["pos"][order]
        corners = rank[corners]
    else:
        pos = np.zeros((0, 3))
        corners = np.zeros(0, dtype=np.int64)
    out = Arrays(np.asarray(pos, dtype=float).reshape(-1, 3),
                 np.asarray(corners, dtype=np.int64),
                 np.asarray(a["ring_sizes"], dtype=np.int64),
                 np.asarray(a["ring_counts"], dtype=np.int64), model_f64=True)
    out.edge_face_counts = np.diff(a["ef_start"])
    return out


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

    @staticmethod
    def defer(mesh: StoreMesh, build, nfaces: int, source=None) -> bool:
        """Turn ``mesh`` into a deferred one that runs ``build(mesh)`` on
        first touch and makes exactly ``nfaces`` faces; ``source`` is what
        it is built from, for :mod:`core.lazy_mesh` (kept until built).
        Only an empty StoreMesh can accept a deferred build."""
        d = mesh.__dict__
        if type(mesh) is not StoreMesh or any(mesh._store.counts()):
            return False
        for name in STORE_SIDE:
            d.pop(name, None)
        d["_deferred"] = build
        d["_deferred_nfaces"] = nfaces
        d["_deferred_source"] = source
        mesh.__class__ = DeferredMesh
        return True

    def _materialize(self) -> None:
        """First touch: a store-backed mesh when the store is on
        (``core.store_mesh.from_deferred``), else the Python objects the
        loader's build makes."""
        if not from_deferred(self):
            self._build_from_callback()
        observe = _mesh.on_mesh_materialized.get()
        if observe is not None:
            observe(self)

    def _build_from_callback(self):
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
        self.__class__ = StoreMesh
        initialise_store(self)
        build(self)

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
