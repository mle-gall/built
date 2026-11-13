# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A :class:`~core.mesh.Mesh` whose geometry lives in the Rust ``MeshStore``
(port step 1 of ``docs/meshstore-design.md``: snapshots, construction and
the lazy-load path).

Why: a Python mesh costs ~2 KB per face in ``Vertex`` / ``Edge`` / ``Face``
objects, and every command snapshots the whole mesh it edits
(``capture_state`` builds a dozen dicts over every element: 0.94 s on the
ride's largest mesh, 0.27 s to restore). A store holds ~220 B per face and
snapshots by sharing ``Arc`` chunks, in microseconds.

What is store-backed (``StoreMesh``):

- a deferred mesh (``DeferredMesh``, a lazy ``.igz`` mesh) on its first
  touch: built from its source arrays by ``MeshStore.from_block``, which is
  ``lazy_mesh.plan`` index for index, i.e. the mesh the Python build makes;
- a mesh built by the bulk constructors on an EMPTY mesh (``bulk_weld`` then
  ``add_edges_welded`` then ``add_faces_welded``, or ``add_faces_bulk``):
  imports, groups, copies of big meshes;
- deep copies and snapshots of those.

Reading goes through view objects (:class:`VertexView`, :class:`EdgeView`,
:class:`FaceView`): ``(mesh, key)`` pairs that subclass the Python element
classes (``isinstance`` checks keep working) and compare and hash BY KEY.
Two views of one element are equal, never identical (no interning). The
writes that change no topology stay in the store: edge ``soft`` /
``hidden`` / ``curve`` / ``layer``, face ``interior``, face ``attrs`` (a
per-face dict once a face's attrs are touched, the shared table entry until
then), ``move_vertex`` / ``place_vertex``.

**Materialise on write.** Every other mutation (``add_face``, ``remove_edge``,
``split_edge``, the stitch, a direct ``loop.reverse()``...) first turns the
mesh into Python objects carrying the store's keys
(:meth:`StoreMesh._materialise`) and then runs the Python kernel unchanged:
the mesh becomes a :class:`MaterialisedMesh`, a plain mesh that also
resolves views handed in from before. Its objects are kept per key
(``_objmap``) and reused by the next materialisation, so a command that
holds an element across an undo that went back to the store still holds the
very object the mesh lists again. Later port steps move those ops into the
store and delete this path.

**Snapshots.** ``capture_state`` of a store-backed mesh is a
:class:`StoreState` (a store snapshot plus the per-face attrs dicts):
O(chunks). Restoring one turns a materialised mesh back into a store-backed
one of the same lineage; restoring a Python snapshot turns a store-backed
mesh into Python objects. One owner at any time: never a store and Python
objects kept in sync.

Switch: ``INGETRAZO_STORE=0/1``, else the General setting ``mesh_store``
(:func:`core._native.store_wanted`); the Rust engine must be on and its
module must have ``MeshStore``. Off, nothing here is reached."""
from __future__ import annotations

from collections.abc import Mapping
from itertools import count

import numpy as np
from PySide6.QtGui import QVector3D

from core import mesh as _mesh
from core.mesh import Edge, Face, Mesh, Vertex

LOCAL_MASK = (1 << 40) - 1

#: ``mesh.__dict__`` entries that belong to the store side and never travel
#: with a deep copy of the Python state.
STORE_SIDE = ("_store", "_objmap", "_attr_table", "_attr_own", "_bulk",
              "_py_base")


def enabled() -> bool:
    """Whether new deferred and bulk-built meshes go to the store."""
    from core import _native
    if not _native.store_wanted():
        return False
    mod = _native.native
    return mod is not None and hasattr(mod, "MeshStore") and hasattr(
        mod.MeshStore, "face_rings")


def _module():
    from core import _native
    return _native.native


def _weld_args():
    return dict(scale=_mesh._KEY_SCALE,
                tol2=(10.0 ** -_mesh._KEY_DECIMALS) ** 2)


def _copy_attrs(d) -> dict:
    """A face's own attrs from a table entry: one level deep, as
    ``formats.igz._face_attrs_from_json`` gives every face its own colour
    list and texture dict."""
    if not d:
        return {}
    return {k: (dict(v) if type(v) is dict else list(v) if type(v) is list
                else v) for k, v in d.items()}


# ---- Materialised element classes -----------------------------------------
# Hashed by key, so a view (hashed by key) finds the object it names in a
# set or a dict and the other way round. Equality stays identity (C speed
# for list scans); a view's own __eq__ answers by key from either side.

def _plain_reduce(base):
    def __reduce_ex__(self, protocol):
        # A copy is a new element of a new mesh (2.1), hashed by identity
        # like any Python element: copy.deepcopy rebuilds a vertex's edge
        # set before the copied edges have their keys, which a key hash
        # could not survive.
        rv = object.__reduce_ex__(self, 2)
        return (rv[0], (base,) + tuple(rv[1][1:])) + tuple(rv[2:])
    return __reduce_ex__


class _MVertex(Vertex):
    __slots__ = ()
    __reduce_ex__ = _plain_reduce(Vertex)

    def __hash__(self):
        return hash(self.key)


class _MEdge(Edge):
    __slots__ = ()
    __reduce_ex__ = _plain_reduce(Edge)

    def __hash__(self):
        return hash(self.key)


class _MFace(Face):
    __slots__ = ()
    __reduce_ex__ = _plain_reduce(Face)

    def __hash__(self):
        return hash(self.key)


# ---- Views ----------------------------------------------------------------

def _store_of(m):
    """The live store of a store-backed mesh, else None."""
    return m.__dict__["_store"] if m.__class__ is StoreMesh else None


class _ViewMixin:
    __slots__ = ()

    def _obj(self):
        """The Python object this view names (materialising the mesh)."""
        m = self._m
        if m.__class__ is StoreMesh:
            m._materialise()
        obj = m.__dict__.get("_objmap", {}).get(self.key)
        if obj is None:
            raise KeyError(f"element {self.key} is not in this mesh")
        return obj

    def _stale(self):
        """A key the store no longer has (a restore across lineages): the
        object a materialisation made for it, as a held Python element
        would answer."""
        obj = self._m.__dict__.get("_objmap", {}).get(self.key)
        if obj is None:
            raise KeyError(f"element {self.key} is not in this mesh")
        return obj

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

    def __reduce__(self):
        # deepcopy / pickle of a view: the element it names.
        return (_identity, (self._obj(),))


def _identity(x):
    return x


class VertexView(_ViewMixin, Vertex):
    __slots__ = ("_m",)

    def __init__(self, m, key):
        self._m = m
        self.key = key

    @property
    def position(self):
        st = _store_of(self._m)
        if st is None:
            return self._obj().position
        try:
            return QVector3D(*st.vertex_position(self.key))
        except KeyError:
            return self._stale().position

    @position.setter
    def position(self, value):
        self._obj().position = value

    @property
    def edges(self):
        m = self._m
        st = _store_of(m)
        if st is None:
            return self._obj().edges
        try:
            keys = st.vertex_edges(self.key)
        except KeyError:
            return self._stale().edges
        return _MutSet(self, "edges", (EdgeView(m, k) for k in keys))

    @edges.setter
    def edges(self, value):
        self._obj().edges = _resolve(value, self._m)


class EdgeView(_ViewMixin, Edge):
    __slots__ = ("_m",)

    def __init__(self, m, key):
        self._m = m
        self.key = key

    def _row(self):
        return _store_of(self._m).edge(self.key)

    @property
    def v0(self):
        m = self._m
        st = _store_of(m)
        if st is None:
            return self._obj().v0
        try:
            return VertexView(m, st.edge_ends(self.key)[0])
        except KeyError:
            return self._stale().v0

    @v0.setter
    def v0(self, value):
        self._obj().v0 = _resolve(value, self._m)

    @property
    def v1(self):
        m = self._m
        st = _store_of(m)
        if st is None:
            return self._obj().v1
        try:
            return VertexView(m, st.edge_ends(self.key)[1])
        except KeyError:
            return self._stale().v1

    @v1.setter
    def v1(self, value):
        self._obj().v1 = _resolve(value, self._m)

    @property
    def faces(self):
        m = self._m
        st = _store_of(m)
        if st is None:
            return self._obj().faces
        try:
            keys = st.edge_faces(self.key)
        except KeyError:
            return self._stale().faces
        return _MutList(self, ("faces",), [FaceView(m, k) for k in keys])

    @faces.setter
    def faces(self, value):
        self._obj().faces = list(_resolve(value, self._m))

    def _flag(self, name):
        st = _store_of(self._m)
        if st is None:
            return getattr(self._obj(), name)
        try:
            _a, _b, flags, curve, layer = st.edge(self.key)
        except KeyError:
            return getattr(self._stale(), name)
        if name == "soft":
            return bool(flags & 1)
        if name == "hidden":
            return bool(flags & 2)
        if name == "curve":
            return None if curve < 0 else curve
        return layer

    def _set_flag(self, name, value):
        m = self._m
        st = _store_of(m)
        if st is None:
            setattr(self._obj(), name, value)
            return
        if name in ("soft", "hidden"):
            flags = st.edge(self.key)[2]
            bit = 1 if name == "soft" else 2
            flags = (flags | bit) if value else (flags & ~bit)
            if not isinstance(value, bool) and value not in (0, 1):
                # A non-bool flag (never written by the app): Python keeps
                # the object; only a Python edge can hold it.
                setattr(self._obj(), name, value)
                return
            st.set_edge_flags(self.key, flags)
        elif name == "curve":
            if value is not None and not isinstance(value, int):
                setattr(self._obj(), name, value)
                return
            st.set_edge_curve(self.key, value)
        else:
            if value is not None and not isinstance(value, str):
                setattr(self._obj(), name, value)
                return
            st.set_edge_layer(self.key, value)

    soft = property(lambda self: self._flag("soft"),
                    lambda self, v: self._set_flag("soft", v))
    hidden = property(lambda self: self._flag("hidden"),
                      lambda self, v: self._set_flag("hidden", v))
    curve = property(lambda self: self._flag("curve"),
                     lambda self, v: self._set_flag("curve", v))
    layer = property(lambda self: self._flag("layer"),
                     lambda self, v: self._set_flag("layer", v))

    @property
    def a(self) -> QVector3D:
        return self.v0.position

    @property
    def b(self) -> QVector3D:
        return self.v1.position

    def other(self, v):
        """The endpoint that is not ``v`` (by key: views are never
        identical)."""
        v0 = self.v0
        return self.v1 if v.key == v0.key else v0

    def has(self, v) -> bool:
        return v.key == self.v0.key or v.key == self.v1.key

    def length(self) -> float:
        st = _store_of(self._m)
        if st is None:
            return self._obj().length()
        a, b = st.edge_ends(self.key)
        p = QVector3D(*st.vertex_position(a))
        q = QVector3D(*st.vertex_position(b))
        return (q - p).length()


def _newell(ring) -> QVector3D:
    """``Face._newell`` over a ring of ``(x, y, z)`` float32 values held as
    doubles: the same sums, so the same bits."""
    count_ = len(ring)
    nx = ny = nz = 0.0
    for i in range(count_):
        x0, y0, z0 = ring[i]
        x1, y1, z1 = ring[(i + 1) % count_]
        nx += (y0 - y1) * (z0 + z1)
        ny += (z0 - z1) * (x0 + x1)
        nz += (x0 - x1) * (y0 + y1)
    return QVector3D(nx, ny, nz)


class FaceView(_ViewMixin, Face):
    __slots__ = ("_m",)

    def __init__(self, m, key):
        self._m = m
        self.key = key

    def _rings(self):
        m = self._m
        return [[VertexView(m, k) for k in r]
                for r in _store_of(m).face_rings(self.key)]

    @property
    def loop(self):
        m = self._m
        st = _store_of(m)
        if st is None:
            return self._obj().loop
        try:
            rings = st.face_rings(self.key)
        except KeyError:
            return self._stale().loop
        return _MutList(self, ("loop",), [VertexView(m, k) for k in rings[0]])

    @loop.setter
    def loop(self, value):
        self._obj().loop = list(_resolve(value, self._m))

    @property
    def hole_loops(self):
        m = self._m
        st = _store_of(m)
        if st is None:
            return self._obj().hole_loops
        try:
            rings = st.face_rings(self.key)
        except KeyError:
            return self._stale().hole_loops
        return _MutList(self, ("hole_loops",), [
            _MutList(self, ("hole_loops", i),
                     [VertexView(m, k) for k in r])
            for i, r in enumerate(rings[1:])])

    @hole_loops.setter
    def hole_loops(self, value):
        self._obj().hole_loops = [list(h) for h in _resolve(value, self._m)]

    def _positions(self):
        st = _store_of(self._m)
        try:
            return st.face_positions(self.key)
        except KeyError:
            return None

    @property
    def vertices(self):
        st = _store_of(self._m)
        rings = self._positions() if st is not None else None
        if rings is None:
            return [v.position for v in self.loop]
        return [QVector3D(*p) for p in rings[0]]

    @property
    def holes(self):
        st = _store_of(self._m)
        rings = self._positions() if st is not None else None
        if rings is None:
            return [[v.position for v in h] for h in self.hole_loops]
        return [[QVector3D(*p) for p in r] for r in rings[1:]]

    @property
    def interior(self):
        st = _store_of(self._m)
        if st is None:
            return self._obj().interior
        try:
            return bool(st.face_info(self.key)[1] & 1)
        except KeyError:
            return self._stale().interior

    @interior.setter
    def interior(self, value):
        st = _store_of(self._m)
        if st is None or not isinstance(value, bool):
            self._obj().interior = value
            return
        flags = st.face_info(self.key)[1]
        st.set_face_flags(self.key, (flags | 1) if value else (flags & ~1))

    @property
    def attrs(self):
        m = self._m
        st = _store_of(m)
        if st is None:
            return self._obj().attrs
        own = m.__dict__["_attr_own"]
        d = own.get(self.key)
        if d is None:
            try:
                idx = st.face_info(self.key)[0]
            except KeyError:
                return self._stale().attrs
            # The face's own dict from now on, as a Python face has one: a
            # write into it (or into a texture dict inside) stays this
            # face's, and the next read hands back the same object.
            d = own[self.key] = _copy_attrs(
                m.__dict__["_attr_table"][idx]) if idx >= 0 else {}
        return d

    @attrs.setter
    def attrs(self, value):
        m = self._m
        if _store_of(m) is None:
            self._obj().attrs = value
            return
        m.__dict__["_attr_own"][self.key] = value

    # Geometry from one fetch of the rings' positions.
    def _newell(self) -> QVector3D:
        rings = self._positions() if _store_of(self._m) is not None else None
        if rings is None:
            return Face._newell(self)
        return _newell(rings[0])

    def normal(self) -> QVector3D:
        rings = self._positions() if _store_of(self._m) is not None else None
        if rings is None:
            return Face.normal(self)
        if len(rings[0]) < 3:
            return QVector3D(0.0, 0.0, 1.0)
        n = _newell(rings[0])
        length = n.length()
        if length < 1e-9:
            return QVector3D(0.0, 0.0, 1.0)
        return n / length

    def area(self) -> float:
        rings = self._positions() if _store_of(self._m) is not None else None
        if rings is None:
            return Face.area(self)
        if len(rings[0]) < 3:
            return 0.0
        return 0.5 * _newell(rings[0]).length()

    def net_area(self) -> float:
        rings = self._positions() if _store_of(self._m) is not None else None
        if rings is None:
            return Face.net_area(self)
        if len(rings[0]) < 3:
            return 0.0
        area = 0.5 * _newell(rings[0]).length()
        for hole in rings[1:]:
            if len(hole) < 3:
                continue
            area -= 0.5 * _newell(hole).length()
        return max(area, 0.0)

    def centroid(self) -> QVector3D:
        rings = self._positions() if _store_of(self._m) is not None else None
        if rings is None:
            return Face.centroid(self)
        loop = rings[0]
        n = len(loop)
        if n == 0:
            return QVector3D(0.0, 0.0, 0.0)
        # Face.centroid sums float32 QVector3D components as doubles.
        cx = sum(p[0] for p in loop) / n
        cy = sum(p[1] for p in loop) / n
        cz = sum(p[2] for p in loop) / n
        return QVector3D(cx, cy, cz)


_VIEW_TYPES = (VertexView, EdgeView, FaceView)
_BASE = {VertexView: Vertex, EdgeView: Edge, FaceView: Face}


def _resolve(x, m):
    """``x`` with every view of ``m`` replaced by the Python object it
    names (materialising ``m``), containers rebuilt; anything else as is."""
    c = x.__class__
    if c is VertexView or c is EdgeView or c is FaceView:
        return x._obj() if x._m is m else x
    if c is list or c is _MutList:
        return [_resolve(i, m) for i in x]
    if c is tuple:
        return tuple(_resolve(i, m) for i in x)
    if c is set or c is _MutSet:
        return {_resolve(i, m) for i in x}
    if c is frozenset:
        return frozenset(_resolve(i, m) for i in x)
    if c is dict:
        return {k: _resolve(v, m) for k, v in x.items()}
    return x


class _MutList(list):
    """A list a view hands out (``edge.faces``, ``face.loop``, a hole). It
    reads like the Python list; an in-place write materialises the mesh and
    is made on the element's real list too, so a caller that edits it
    (``f.loop.reverse()``, ``edge.faces.append(f)``) edits the mesh, as it
    does on a Python mesh."""

    __slots__ = ("_owner", "_path")

    def __init__(self, owner, path, items):
        list.__init__(self, items)
        self._owner = owner
        self._path = path

    def _real(self):
        obj = self._owner._obj()
        target = getattr(obj, self._path[0])
        for i in self._path[1:]:
            target = target[i]
        return target

    def _fwd(name):
        base = getattr(list, name)

        def method(self, *args):
            m = self._owner._m
            real = self._real()
            base(real, *_resolve(args, m))
            return base(self, *args)
        method.__name__ = name
        return method

    append = _fwd("append")
    extend = _fwd("extend")
    insert = _fwd("insert")
    remove = _fwd("remove")
    pop = _fwd("pop")
    clear = _fwd("clear")
    reverse = _fwd("reverse")
    __setitem__ = _fwd("__setitem__")
    __delitem__ = _fwd("__delitem__")

    def sort(self, *args, **kw):
        real = self._real()
        real.sort(*args, **kw)
        list.sort(self, *args, **kw)

    def __iadd__(self, other):
        self.extend(other)
        return self

    def __imul__(self, n):
        real = self._real()
        real *= n
        list.__imul__(self, n)
        return self

    del _fwd


class _MutSet(set):
    """``vertex.edges`` from a view: a set, written through like
    :class:`_MutList`."""

    __slots__ = ("_owner", "_attr")

    def __init__(self, owner, attr, items):
        set.__init__(self, items)
        self._owner = owner
        self._attr = attr

    def _fwd(name):
        base = getattr(set, name)

        def method(self, *args):
            m = self._owner._m
            real = getattr(self._owner._obj(), self._attr)
            base(real, *_resolve(args, m))
            return base(self, *args)
        method.__name__ = name
        return method

    add = _fwd("add")
    discard = _fwd("discard")
    remove = _fwd("remove")
    pop = _fwd("pop")
    clear = _fwd("clear")
    update = _fwd("update")
    difference_update = _fwd("difference_update")
    intersection_update = _fwd("intersection_update")
    symmetric_difference_update = _fwd("symmetric_difference_update")

    def __ior__(self, other):
        self.update(other)
        return self

    def __isub__(self, other):
        self.difference_update(other)
        return self

    def __iand__(self, other):
        self.intersection_update(other)
        return self

    def __ixor__(self, other):
        self.symmetric_difference_update(other)
        return self

    del _fwd


class _Seq:
    """``mesh.vertices`` / ``edges`` / ``faces`` of a store-backed mesh: a
    live list of views in list order. Kept past a materialisation (an alias
    taken before), it reads and writes the mesh's real list."""

    __slots__ = ("_m", "_kind", "_name", "_cls", "_idx")

    def __init__(self, m, kind, name, cls, idx):
        self._m = m
        self._kind = kind
        self._name = name
        self._cls = cls
        self._idx = idx

    def _real(self):
        m = self._m
        if m.__class__ is StoreMesh:
            return None
        return m.__dict__[self._name]

    def _keys(self):
        return self._m.__dict__["_store"].keys(self._kind)

    def __len__(self):
        real = self._real()
        if real is not None:
            return len(real)
        return self._m.__dict__["_store"].counts()[self._idx]

    def __bool__(self):
        return len(self) > 0

    def __iter__(self):
        real = self._real()
        if real is not None:
            return iter(real)
        m, cls = self._m, self._cls
        return (cls(m, k) for k in self._keys())

    def __reversed__(self):
        return reversed(list(self))

    def __getitem__(self, i):
        real = self._real()
        if real is not None:
            return real[i]
        if isinstance(i, slice):
            m, cls = self._m, self._cls
            return [cls(m, k) for k in self._keys()[i]]
        st = self._m.__dict__["_store"]
        n = st.counts()[self._idx]
        if i < 0:
            i += n
        if not 0 <= i < n:
            raise IndexError("list index out of range")
        return self._cls(self._m, st.key_at(self._kind, i))

    def __contains__(self, x):
        real = self._real()
        if real is not None:
            return x in real
        if not isinstance(x, _BASE[self._cls]):
            return False
        try:
            return self._m.__dict__["_store"].is_live(x.key)
        except (OverflowError, TypeError):
            return False

    def index(self, x, *args):
        real = self._real()
        if real is not None:
            return real.index(x, *args)
        if not args and x in self:
            return self._m.__dict__["_store"].index_of(x.key)
        return list(self).index(x, *args)

    def count(self, x):
        real = self._real()
        if real is not None:
            return real.count(x)
        return 1 if x in self else 0

    def copy(self):
        return list(self)

    def __eq__(self, other):
        if isinstance(other, (list, _Seq)):
            return list(self) == list(other)
        return NotImplemented

    def __add__(self, other):
        return list(self) + list(other)

    def __radd__(self, other):
        return list(other) + list(self)

    def __repr__(self):
        return repr(list(self))

    def _write(name):
        def method(self, *args):
            m = self._m
            if m.__class__ is StoreMesh:
                m._materialise()
            real = m.__dict__[self._name]
            return getattr(real, name)(*_resolve(args, m))
        method.__name__ = name
        return method

    append = _write("append")
    extend = _write("extend")
    insert = _write("insert")
    remove = _write("remove")
    pop = _write("pop")
    clear = _write("clear")
    reverse = _write("reverse")
    sort = _write("sort")
    __setitem__ = _write("__setitem__")
    __delitem__ = _write("__delitem__")

    def __iadd__(self, other):
        self.extend(other)
        return self

    del _write


# ---- Snapshots --------------------------------------------------------------

class StoreState(Mapping):
    """``capture_state()`` of a store-backed mesh: the store's snapshot
    (O(chunks)), the faces' own attrs dicts (copied, as Python copies
    ``f.attrs``) and the attribute table they index.

    It reads like the Python snapshot dict for the few callers that peek
    (``snap["vpos"]`` for "was this vertex there", ``snap["faces"]`` /
    ``["edges"]`` as elements of the mesh); every other entry is built on
    demand from the snapshot's arrays."""

    __slots__ = ("snap", "own", "table", "mesh", "_cache")

    def __init__(self, snap, own, table, mesh):
        self.snap = snap
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
        """``==`` against a Python snapshot (the mesh materialised since):
        entry by entry, views against the objects of the same keys."""
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
             "efaces", "esoft", "ehidden", "elayer", "ecurve", "floop",
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
        hit = self._cache[name] = self._build(name)
        return hit

    def _build(self, name):
        snap, m = self.snap, self.mesh
        if name == "vpos":
            return _SnapPositions(snap)
        if name in ("vertices", "edges", "faces"):
            kind = name[:-1] if name != "vertices" else "vertex"
            cls = {"vertex": VertexView, "edge": EdgeView,
                   "face": FaceView}[kind]
            return [cls(m, k) for k in snap.keys(kind)]
        a = snap.arrays()
        vk = snap.keys("vertex")
        ek = snap.keys("edge")
        fk = snap.keys("face")
        V = [VertexView(m, k) for k in vk]
        F = [FaceView(m, k) for k in fk]
        if name == "registry":
            cells, vid = snap.registry()
            return {tuple(c): V[i] for c, i in zip(cells.tolist(),
                                                   vid.tolist())}
        if name == "vedges":
            out = {k: set() for k in vk}
            for e, (i, j) in zip(ek, a["edges"].tolist()):
                out[vk[i]].add(EdgeView(m, e))
                out[vk[j]].add(EdgeView(m, e))
            return out
        if name == "efaces":
            s = a["ef_start"].tolist()
            ff = a["ef_face"].tolist()
            return {e: [F[j] for j in ff[s[i]:s[i + 1]]]
                    for i, e in enumerate(ek)}
        fl = a["flags"].tolist()
        if name == "esoft":
            return {e: bool(f & 1) for e, f in zip(ek, fl)}
        if name == "ehidden":
            return {e: bool(f & 2) for e, f in zip(ek, fl)}
        if name == "ecurve":
            return {e: (None if c < 0 else c)
                    for e, c in zip(ek, a["curve"].tolist())}
        if name == "elayer":
            names = snap.layer_names()
            return {e: (None if c < 0 else names[c])
                    for e, c in zip(ek, a["layer"].tolist())}
        corners, sizes = a["corners"].tolist(), a["ring_sizes"].tolist()
        counts_ = a["ring_counts"].tolist()
        if name in ("floop", "fholes"):
            out = {}
            c = r = 0
            for f, nr in zip(fk, counts_):
                loops = []
                for _ in range(nr):
                    loops.append([V[j] for j in corners[c:c + sizes[r]]])
                    c += sizes[r]
                    r += 1
                out[f] = loops[0] if name == "floop" else loops[1:]
            return out
        if name == "fattrs":
            return {f: dict(self._face_attrs(f, i))
                    for f, i in zip(fk, a["face_attr"].tolist())}
        return {f: bool(x & 1) for f, x in zip(fk, a["face_flags"].tolist())}


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

def _seq_property(kind, name, cls, idx):
    def get(self):
        return _Seq(self, kind, name, cls, idx)

    def put(self, value):
        self._materialise()
        setattr(self, name, value)
    return property(get, put)


def _materialising(name):
    def get(self):
        self._materialise()
        return getattr(self, name)

    def put(self, value):
        self._materialise()
        setattr(self, name, value)
    return property(get, put)


def _on_python(name):
    """A Mesh method the store cannot do yet: materialise, then run the
    Python kernel (through MaterialisedMesh, which resolves views)."""
    def method(self, *args, **kw):
        self._materialise()
        return getattr(self, name)(*args, **kw)
    method.__name__ = name
    method.__doc__ = f"Materialise, then :meth:`Mesh.{name}`."
    return method


class StoreMesh(Mesh):
    """A mesh whose geometry is a ``MeshStore`` (see the module docstring).
    Never constructed directly: a :class:`Mesh` becomes one
    (:func:`become_store`)."""

    vertices = _seq_property("vertex", "vertices", VertexView, 0)
    edges = _seq_property("edge", "edges", EdgeView, 1)
    faces = _seq_property("face", "faces", FaceView, 2)
    _registry = _materialising("_registry")
    _displaced = _materialising("_displaced")

    # -- what the store answers itself
    def _bump(self):
        self._chunk_dirty = True
        self._mut_serial += 1
        self.__dict__.pop("_bulk", None)

    def vertex_at(self, position):
        k = self._store.vertex_at(_xyz(position))
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

    def purge_debris(self):
        """Nothing to drop is answered from the arrays; a store has no
        ghost entries (design decision 4), so only an unreferenced vertex
        (an edge welded shut at build) needs the Python sweep."""
        a = self._store.arrays()
        nv = len(a["pos"])
        used = np.zeros(nv, dtype=bool)
        if len(a["edges"]):
            used[a["edges"].ravel()] = True
        if len(a["corners"]):
            used[a["corners"]] = True
        if used.all():
            return 0, 0
        self._materialise()
        return self.purge_debris()

    def prune_orphan_vertices(self) -> int:
        return self.purge_debris()[0]

    def resplit_curves(self) -> None:
        """:meth:`Mesh.resplit_curves` with the scan for curve edges done
        on the store's arrays; the regrouping walks views and writes the
        new ids through them."""
        self._chunk_dirty = True
        self._mut_serial += 1
        st = self._store
        curve = st.arrays()["curve"]
        hit = np.flatnonzero(curve >= 0)
        if not len(hit):
            return
        keys = st.keys("edge")
        by_id: dict = {}
        for i, c in zip(hit.tolist(), curve[hit].tolist()):
            by_id.setdefault(c, []).append(EdgeView(self, keys[i]))
        Mesh._resplit_curve_groups(by_id)

    def clear(self) -> None:
        d = self.__dict__
        d.update(vertices=[], edges=[], faces=[], _registry={},
                 _displaced={}, _attr_own={})
        d.pop("_bulk", None)
        d.pop("_py_base", None)
        self.__class__ = MaterialisedMesh
        self._chunk_dirty = True
        self._mut_serial += 1

    def capture_state(self):
        d = self.__dict__
        return StoreState(self._store.snapshot(),
                          {k: dict(v) for k, v in d["_attr_own"].items()},
                          d["_attr_table"], self)

    def restore_state(self, snap) -> None:
        if isinstance(snap, StoreState):
            _restore_store_state(self, snap)
            return
        # A Python snapshot: the mesh becomes those objects again.
        _to_python_lists(self)
        Mesh.restore_state(self, snap)

    # -- bulk construction straight into the store
    def bulk_weld(self, pos):
        self._materialise()
        return self.bulk_weld(pos)

    def add_edges_welded(self, vobjs, ia, ib, flags=None):
        b = self.__dict__.get("_bulk")
        if b is None or vobjs is not b["vobjs"] or b["stage"] != "points":
            self._materialise()
            return self.add_edges_welded(vobjs, ia, ib, flags)
        return _bulk_edges(self, b, ia, ib, flags)

    def add_faces_welded(self, vobjs, inverse, ring_sizes, face_ring_counts,
                         attrs=None, edge_map=None):
        b = self.__dict__.get("_bulk")
        if (b is None or vobjs is not b["vobjs"]
                or b["stage"] not in ("points", "edges")
                or not (edge_map is None or isinstance(edge_map, _EdgeMap))):
            self._materialise()
            return self.add_faces_welded(vobjs, inverse, ring_sizes,
                                         face_ring_counts, attrs, edge_map)
        return _bulk_faces(self, b, inverse, ring_sizes, face_ring_counts,
                           attrs)

    # -- copies
    def __deepcopy__(self, memo):
        from copy import deepcopy
        new = StoreMesh.__new__(StoreMesh)
        memo[id(self)] = new
        d = self.__dict__
        rest = {k: v for k, v in d.items()
                if k not in STORE_SIDE and k != "_keys"
                and k not in ("vertices", "edges", "faces", "_registry",
                              "_displaced")}
        new.__dict__.update(deepcopy(rest, memo))
        new.__dict__["_keys"] = count(next(_mesh._KEY_NAMESPACES) << 40)
        ns = next(_mesh._KEY_NAMESPACES)
        new.__dict__["_store"] = d["_store"].copy(ns)
        new.__dict__["_attr_table"] = d["_attr_table"]
        new.__dict__["_attr_own"] = {
            (ns << 40) | (k & LOCAL_MASK): deepcopy(v, memo)
            for k, v in d["_attr_own"].items()}
        return new

    def __reduce_ex__(self, protocol):
        self._materialise()
        return self.__reduce_ex__(protocol)

    # -- materialise on write
    def _materialise(self) -> None:
        """Become Python objects with the store's keys (see the module
        docstring); a no-op once done."""
        if self.__class__ is not StoreMesh:
            return
        _materialise(self)


for _name in ("_probe_neighbours", "vertex", "_link_edge", "add_edge", "remove_edge", "relink_vertices",
              "drop_orphan_vertices", "add_faces_bulk", "add_face",
              "remove_face", "relink_edge", "relink_face", "add_hole",
              "remove_hole", "_rekey", "split_edge", "weld_coincident",
              "dedupe_faces", "_weld_into", "split_edge_at",
              "collapse_vertex", "dissolve_edge_recorded",
              "dissolve_coplanar_pair", "dissolve_coplanar_region",
              "revert_stitch", "dissolve_edge"):
    setattr(StoreMesh, _name, _on_python(_name))
# add_faces_bulk: bulk_weld + add_faces_welded on an empty mesh goes to the
# store through Mesh.add_faces_bulk itself (become_store at bulk_weld).
StoreMesh.add_faces_bulk = Mesh.add_faces_bulk
# Read-mostly Python code that runs on views as is (curve ids are written
# through the edge views): tag_curve, resplit_curves, curve_edges,
# surface_of, interior_vertex_on, collapsible_vertex.
StoreMesh.tag_curve = Mesh.tag_curve


def _xyz(p):
    if isinstance(p, QVector3D):
        return p.toTuple()
    x, y, z = p
    return (float(x), float(y), float(z))


def _resolving(name):
    base = getattr(Mesh, name)

    def method(self, *args, **kw):
        return base(self, *_resolve(args, self),
                    **{k: _resolve(v, self) for k, v in kw.items()})
    method.__name__ = name
    method.__doc__ = base.__doc__
    return method


class MaterialisedMesh(Mesh):
    """A store-backed mesh turned into Python objects: a plain mesh, plus
    the views handed out before resolved to the objects they name, and the
    store kept so a store snapshot can be restored (back to store-backed)."""

    def find_edge(self, v0, v1):
        if v0.__class__ is VertexView:
            v0 = _resolve(v0, self)
        if v1.__class__ is VertexView:
            v1 = _resolve(v1, self)
        return Mesh.find_edge(self, v0, v1)

    def move_vertex(self, v, delta) -> None:
        if v.__class__ is VertexView:
            v = _resolve(v, self)
        Mesh.move_vertex(self, v, delta)

    def place_vertex(self, v, position) -> None:
        if v.__class__ is VertexView:
            v = _resolve(v, self)
        Mesh.place_vertex(self, v, position)

    def capture_state(self):
        return Mesh.capture_state(self)

    def restore_state(self, snap) -> None:
        if isinstance(snap, StoreState):
            _restore_store_state(self, snap)
            return
        Mesh.restore_state(self, snap)



for _name in ("curve_edges", "remove_edge", "relink_vertices",
              "drop_orphan_vertices", "add_edges_welded", "add_faces_welded",
              "surface_of", "remove_face", "relink_edge", "relink_face",
              "add_hole", "remove_hole", "edges_at", "faces_at", "registers",
              "split_edge", "_weld_into", "interior_vertex_on",
              "split_edge_at", "collapsible_vertex", "collapse_vertex",
              "dissolve_edge_recorded", "dissolve_coplanar_pair",
              "dissolve_coplanar_region", "revert_stitch", "dissolve_edge",
              "_rekey", "_link_edge"):
    setattr(MaterialisedMesh, _name, _resolving(_name))
del _name


# ---- Building, materialising, restoring -------------------------------------

def _to_python_lists(m) -> None:
    """Make ``m`` Python-backed with empty lists (a Python snapshot is about
    to define it completely)."""
    d = m.__dict__
    d.update(vertices=[], edges=[], faces=[], _registry={}, _displaced={},
             _attr_own={})
    d.pop("_bulk", None)
    d.pop("_py_base", None)
    m.__class__ = MaterialisedMesh


def _restore_store_state(m, snap: StoreState) -> None:
    """``m.restore_state(snap)`` for a store snapshot: ``m`` is store-backed
    afterwards. Same lineage: the store goes back to it, keys kept. Another
    store's snapshot (``Scene.share_back``) is adopted under a fresh
    namespace: new keys (design decision 3)."""
    d = m.__dict__
    st = d.get("_store")
    foreign = st is None or st.lineage != snap.snap.lineage
    if st is None:
        st = d["_store"] = _module().MeshStore(
            next(_mesh._KEY_NAMESPACES), **_weld_args())
    if foreign:
        ns = next(_mesh._KEY_NAMESPACES)
        st.restore(snap.snap, namespace=ns)
        own = {(ns << 40) | (k & LOCAL_MASK): dict(v)
               for k, v in snap.own.items()}
    else:
        st.restore(snap.snap)
        own = {k: dict(v) for k, v in snap.own.items()}
    d["_attr_table"] = snap.table
    d["_attr_own"] = own
    d.setdefault("_objmap", {})
    for name in ("vertices", "edges", "faces", "_registry", "_displaced",
                 "_bulk"):
        d.pop(name, None)
    m.__class__ = StoreMesh
    m._chunk_dirty = True
    m._mut_serial += 1


def become_store(m, store, table, own=None) -> None:
    """Make the plain, empty ``m`` store-backed by ``store``."""
    d = m.__dict__
    for name in ("vertices", "edges", "faces", "_registry", "_displaced"):
        d.pop(name, None)
    d["_store"] = store
    d["_attr_table"] = table
    d["_attr_own"] = own if own is not None else {}
    d.setdefault("_objmap", {})
    m.__class__ = StoreMesh


def _materialise(m) -> None:
    """Python objects from the store, keys kept, objects reused per key.

    The collector is paused meanwhile, as the loader does for its mass
    construction: on the ride's largest mesh (1.07M elements) the
    generational passes over the growing heap were 2.4 s of 3.0 s."""
    import gc
    was = gc.isenabled()
    gc.disable()
    try:
        _materialise_objects(m)
    finally:
        if was:
            gc.enable()


def _materialise_objects(m) -> None:
    d = m.__dict__
    st = d["_store"]
    a = st.arrays()
    vk = st.keys("vertex")
    ek = st.keys("edge")
    fk = st.keys("face")
    objmap = d.setdefault("_objmap", {})
    table = d["_attr_table"]
    own = d["_attr_own"]
    get = objmap.get
    new_v = _MVertex.__new__
    verts = []
    append = verts.append
    for k, p in zip(vk, a["pos"].tolist()):
        v = get(k)
        if v is None:
            v = new_v(_MVertex)
            v.key = k
            objmap[k] = v
        v.position = QVector3D(p[0], p[1], p[2])
        v.edges = set()
        append(v)
    names = st.layer_names()
    new_e = _MEdge.__new__
    edges = []
    append = edges.append
    for k, (i0, i1), fl, c, la in zip(ek, a["edges"].tolist(),
                                      a["flags"].tolist(),
                                      a["curve"].tolist(),
                                      a["layer"].tolist()):
        e = get(k)
        if e is None:
            e = new_e(_MEdge)
            e.key = k
            objmap[k] = e
        v0 = verts[i0]
        v1 = verts[i1]
        e.v0 = v0
        e.v1 = v1
        e.faces = []
        e.soft = bool(fl & 1)
        e.hidden = bool(fl & 2)
        e.curve = None if c < 0 else c
        e.layer = None if la < 0 else names[la]
        v0.edges.add(e)
        v1.edges.add(e)
        append(e)
    corners = a["corners"].tolist()
    sizes = a["ring_sizes"].tolist()
    new_f = _MFace.__new__
    faces = []
    append = faces.append
    c = r = 0
    for k, nr, ai, ff in zip(fk, a["ring_counts"].tolist(),
                             a["face_attr"].tolist(),
                             a["face_flags"].tolist()):
        f = get(k)
        if f is None:
            f = new_f(_MFace)
            f.key = k
            objmap[k] = f
        s = sizes[r]
        f.loop = [verts[j] for j in corners[c:c + s]]
        c += s
        r += 1
        holes = []
        for _ in range(nr - 1):
            s = sizes[r]
            holes.append([verts[j] for j in corners[c:c + s]])
            c += s
            r += 1
        f.hole_loops = holes
        o = own.get(k)
        f.attrs = o if o is not None else (
            _copy_attrs(table[ai]) if ai >= 0 else {})
        f.interior = bool(ff & 1)
        append(f)
    start = a["ef_start"].tolist()
    ef = a["ef_face"].tolist()
    for i, e in enumerate(edges):
        s0, s1 = start[i], start[i + 1]
        if s1 > s0:
            e.faces = [faces[j] for j in ef[s0:s1]]
    cells, vid = st.registry()
    registry = dict(zip(map(tuple, cells.tolist()),
                        [verts[i] for i in vid.tolist()]))
    displaced = {cell: [verts[i] for i in q]
                 for cell, q in st.displaced().items()}
    d.update(vertices=verts, edges=edges, faces=faces, _registry=registry,
             _displaced=displaced, _attr_own={})
    d.pop("_bulk", None)
    m.__class__ = MaterialisedMesh


# ---- Construction -----------------------------------------------------------

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


def bulk_weld(m, pos):
    """``Mesh.bulk_weld`` on an empty mesh, into a new store. Returns the
    views of the welded vertices and each point's vertex index, as the
    Python method does."""
    mod = _module()
    pos32 = np.ascontiguousarray(pos, dtype=np.float32).reshape(-1, 3)
    w = _weld_args()
    if not len(pos32):
        return [], np.empty(0, dtype=np.int64)
    _vpos, ids, origin = mod.weld_points(pos32, w["scale"], w["tol2"])
    ns = next(_mesh._KEY_NAMESPACES)
    empty_i = np.zeros(0, dtype=np.int64)
    store = mod.MeshStore.from_block(
        ns, pos32, np.zeros((0, 2), dtype=np.int64), None, None, None, [],
        empty_i, empty_i, empty_i, None, **w)
    become_store(m, store, [])
    vobjs = [VertexView(m, k) for k in store.keys("vertex")]
    m.__dict__["_bulk"] = dict(
        stage="points", vobjs=vobjs, pos=pos32,
        origin=np.asarray(origin, dtype=np.int64), ns=ns,
        edges=np.zeros((0, 2), dtype=np.int64))
    return vobjs, np.asarray(ids, dtype=np.int64)


def _edge_columns(st):
    """The store's edges as the block arrays that rebuild them as they are
    now (flags written through the views since included)."""
    a = st.arrays()
    return (a["edges"], a["flags"], a["curve"], a["layer"],
            list(st.layer_names()))


def _rebuild(m, b, edges_v, flags, curves, layers, names, rings=None,
             sizes=None, counts=None, face_attr=None):
    mod = _module()
    origin = b["origin"]
    pts = origin[edges_v] if len(edges_v) else np.zeros((0, 2), np.int64)
    e = np.ascontiguousarray(pts, dtype=np.int64).reshape(-1, 2)
    empty_i = np.zeros(0, dtype=np.int64)
    store = mod.MeshStore.from_block(
        b["ns"], b["pos"], e,
        np.ascontiguousarray(flags, dtype=np.uint8),
        np.ascontiguousarray(curves, dtype=np.int64),
        np.ascontiguousarray(layers, dtype=np.int64), names,
        empty_i if rings is None else np.ascontiguousarray(
            origin[rings], dtype=np.int64),
        empty_i if sizes is None else sizes,
        empty_i if counts is None else counts,
        face_attr, **_weld_args())
    m.__dict__["_store"] = store
    return store


def _bulk_edges(m, b, ia, ib, flags):
    """``add_edges_welded`` into a fresh bulk store: rebuilt from the same
    points with these edges (``from_block`` is that very sequence)."""
    ia = np.asarray(ia, dtype=np.int64).reshape(-1)
    ib = np.asarray(ib, dtype=np.int64).reshape(-1)
    n = len(ia)
    fl = np.zeros(n, dtype=np.uint8)
    cv = np.full(n, -1, dtype=np.int64)
    lv = np.full(n, -1, dtype=np.int64)
    names: list = []
    name_ix: dict = {}
    if flags is not None:
        for i, row in enumerate(flags):
            soft, curve, layer = row[:3]
            bits = 1 if soft else 0
            if len(row) > 3 and row[3]:
                bits |= 2
            fl[i] = bits
            if curve is not None:
                cv[i] = curve
            if layer is not None:
                if not isinstance(layer, str):
                    # A non-string tag (never written by the app): only
                    # Python edges hold it.
                    _abandon_bulk(m, b)
                    return m.add_edges_welded(b["vobjs"], ia, ib, flags)
                j = name_ix.get(layer)
                if j is None:
                    j = name_ix[layer] = len(names)
                    names.append(layer)
                lv[i] = j
    edges_v = np.stack([ia, ib], axis=1) if n else np.zeros((0, 2), np.int64)
    m._chunk_dirty = True
    m._mut_serial += 1
    st = _rebuild(m, b, edges_v, fl, cv, lv, names)
    b["stage"] = "edges"
    vobjs = b["vobjs"]
    return _EdgeMap(m, vobjs)


def _abandon_bulk(m, b) -> None:
    """Leave the bulk store path: the mesh as Python objects from here."""
    m.__dict__.pop("_bulk", None)
    m._materialise()


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
        _abandon_bulk(m, b)
        return m.add_faces_welded(b["vobjs"], inverse, ring_sizes,
                                  face_ring_counts, attrs)
    st = m.__dict__["_store"]
    edges_v, fl, cv, lv, names = _edge_columns(st)
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
    st = _rebuild(m, b, edges_v, fl, cv, lv, names, inverse, sizes, counts_,
                  fa)
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


def store_from_source(src, hooks=()):
    """The store a deferred mesh's ``_deferred_source`` builds, with its
    attribute table. ``hooks`` are the after-build functions links attached
    (``core.links._retable_deferred``): per-element relabels of attrs and
    edge tags, applied here to the table rows and tag names once instead of
    to every face — the same result, since every face of one row has the
    same attrs."""
    from formats.igz import _face_attrs_from_json
    mod = _module()
    meta = src.get("meta") or {}
    raw = meta.get("face_attrs") or []
    table = [_face_attrs_from_json(row) or {} for row in raw]
    names = list(meta.get("edge_layers") or [])
    n_e = len(src["edges"]) if src.get("edges") is not None else 0
    fa = src.get("face_attr")
    nfaces = len(src["counts"])
    fa = (np.full(nfaces, -1, dtype=np.int64) if fa is None
          else np.asarray(fa, dtype=np.int64))
    layers = src.get("layers")
    layers = None if layers is None else np.asarray(layers, dtype=np.int64)
    if hooks:
        table, fa, names, layers = _apply_hooks(hooks, table, fa, names,
                                                layers, n_e)
    w = _weld_args()
    ns = next(_mesh._KEY_NAMESPACES)
    pos = np.ascontiguousarray(src["pos"], dtype=np.float32).reshape(-1, 3)
    edges = np.ascontiguousarray(src["edges"], dtype=np.int64).reshape(-1, 2)
    flags = src.get("flags")
    curves = src.get("curves")
    store = mod.MeshStore.from_block(
        ns, pos, edges,
        None if flags is None else np.ascontiguousarray(flags, np.uint8),
        None if curves is None else np.ascontiguousarray(curves, np.int64),
        None if layers is None else np.ascontiguousarray(layers, np.int64),
        names,
        np.ascontiguousarray(src["rings"], dtype=np.int64),
        np.ascontiguousarray(src["sizes"], dtype=np.int64),
        np.ascontiguousarray(src["counts"], dtype=np.int64),
        np.ascontiguousarray(fa, dtype=np.int64),
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
    if not enabled():
        return False
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
                "_deferred_hook_source",
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
    own = m.__dict__["_attr_own"]
    table = m.__dict__["_attr_table"]
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


def attr_columns(m, overrides=None):
    """``(table, face_attr)`` of a store-backed mesh for array readers
    (pack, takeoff): every face's attrs as an index into ``table`` (-1:
    none). Faces with their own dict (touched through a view) or an entry
    in ``overrides`` (face key → attrs) get a row of their own; the others
    share the attribute table's rows."""
    d = m.__dict__
    st = d["_store"]
    fa = st.arrays()["face_attr"].copy()
    table = list(d["_attr_table"])
    own = d["_attr_own"]
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
                 np.asarray(a["ring_counts"], dtype=np.int64))
    out.edge_face_counts = np.diff(a["ef_start"])
    return out
