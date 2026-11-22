# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Pieces, Explode and Purge on store-backed meshes (docs/meshstore-design.md,
"Pieces, Explode and Purge"): the parts of ``core.pieces``,
``ExplodeGroupCommand`` and ``core.purge`` that walked every face and edge
in Python, done by the store (``rust/ingetrazo_core/src/store/pieces.rs``,
``explode.rs``) or over its arrays, without building Python objects for the
mesh's elements and without materialising it.

Every function answers ``None`` when it cannot serve the mesh (a Python
mesh, a native module without the op, no QVector3D formula for the
geometric tests); the caller then runs the Python kernel, whose result is
the same."""
from __future__ import annotations

import weakref
from collections.abc import Mapping, Sequence

import numpy as np
from PySide6.QtGui import QVector3D


def _store_mesh(mesh):
    """``mesh`` when it is a live store-backed mesh, else None."""
    from core.store_mesh import StoreMesh
    return mesh if type(mesh) is StoreMesh else None


def _hypot():
    from core.store_mesh import _hypot as h
    return h()


# ---------------------------------------------------------------- pieces

def parts(mesh, solid: bool):
    """``core.pieces.solid_parts`` (``solid``) or ``connected_parts`` of a
    store-backed mesh: ``[(faces, edges)]`` as views, in Python's order."""
    m = _store_mesh(mesh)
    if m is None:
        return None
    snap = m._store.snapshot()
    if not hasattr(snap, "solid_parts"):
        return None
    if solid:
        h = _hypot()
        if h is None:
            return None
        found = snap.solid_parts(h)
    else:
        found = snap.connected_parts()
    from core.store_mesh import EdgeView, FaceView
    return [(Views(m, fs, FaceView), Views(m, es, EdgeView))
            for fs, es in found]


class Views(Sequence):
    """Elements of a store-backed mesh by key, a view made per read: a
    piece of 90,000 faces costs no Python object until one is asked for
    (:func:`copy_part` reads the keys)."""

    __slots__ = ("_m", "_keys", "_cls")

    def __init__(self, m, keys, cls):
        self._m, self._keys, self._cls = m, keys, cls

    def __len__(self):
        return len(self._keys)

    def __getitem__(self, i):
        if isinstance(i, slice):
            return [self._cls(self._m, k) for k in self._keys[i]]
        return self._cls(self._m, self._keys[i])

    def __iter__(self):
        m, cls = self._m, self._cls
        return (cls(m, k) for k in self._keys)

    def __repr__(self):
        return f"Views({len(self._keys)} {self._cls.__name__})"


def _keys_of(elements, cls):
    """``(mesh, keys)`` when every element is a view of one store mesh."""
    if isinstance(elements, Views) and elements._cls is cls:
        return elements._m, list(elements._keys)
    m = _owner(elements, cls)
    return m, ([x.key for x in elements] if m is not None else None)


def _owner(elements, cls):
    """The store mesh every element of ``elements`` is a view of, if one."""
    m = None
    for x in elements:
        if type(x) is not cls:
            return None
        if m is None:
            m = x._m
        elif x._m is not m:
            return None
    return m


def copy_part(faces, edges):
    """``core.pieces.copy_part`` when every face and edge is a view of one
    store-backed mesh: the new mesh is built by the store from the keys
    (``StoreSnapshot.extract``), the attrs rows mapped once per row."""
    from core.store_mesh import (EdgeView, FaceView, _hole_filter,
                                 _table_index, _weld_args, become_store)
    if not len(faces) and not len(edges):
        return None
    m, fkeys = _keys_of(faces, FaceView) if len(faces) else (None, [])
    me, ekeys = _keys_of(edges, EdgeView) if len(edges) else (None, [])
    if fkeys is None or ekeys is None:
        return None
    m = m or me
    if m is None or (len(faces) and len(edges) and me is not m) \
            or _store_mesh(m) is None:
        return None
    snap = m._store.snapshot()
    if not hasattr(snap, "extract"):
        return None
    from core import mesh as _mesh
    d = m.__dict__
    table_src, own = d["_attr_table"], d["_attr_own"]
    # Rows once per row used; a face's own dict, row of its own.
    rows = snap.face_rows(fkeys) if fkeys else np.zeros(0, dtype=np.int64)
    table: list = []
    index: dict = {}
    uniq, inverse = np.unique(rows, return_inverse=True)
    face_attr = np.array([_table_index(table, index, table_src[r])
                          if r >= 0 else -1 for r in uniq.tolist()],
                         dtype=np.int64)[inverse.reshape(-1)] \
        if len(rows) else np.zeros(0, dtype=np.int64)
    if own:
        at = {k: i for i, k in enumerate(fkeys)}
        for k in sorted(own.keys() & at.keys(), key=at.get):
            face_attr[at[k]] = _table_index(table, index, own[k])
    face_attr = face_attr.tolist()
    store = snap.extract(next(_mesh._KEY_NAMESPACES), fkeys, ekeys,
                         face_attr, _hole_filter, **_weld_args())
    new = _mesh.Mesh()
    become_store(new, store, table)
    new.resplit_curves()
    _ORIGINS[new] = (weakref.ref(m),
                     CopiedKeys({"face": (fkeys, store.keys("face"))}))
    return new


#: Piece mesh -> (weak source mesh, its faces' source keys): what
#: :func:`copy_part` cut it from, for the change feed.
_ORIGINS: "weakref.WeakKeyDictionary" = None


def origin_of(mesh):
    """``(source mesh, CopiedKeys)`` for a mesh :func:`copy_part` made from
    a store-backed mesh, else ``(None, None)``."""
    got = _ORIGINS.get(mesh)
    if got is None:
        return None, None
    src = got[0]()
    return (src, got[1]) if src is not None else (None, None)


# ---------------------------------------------------------------- explode

def _needs_face(side) -> bool:
    """Whether ``placed_face_paint`` needs the face itself for this side:
    a texture without its ``uvw`` map, or PBR maps without a texture, are
    placed from the face's normal."""
    if not isinstance(side, dict):
        return False
    tex = side.get("texture")
    if tex:
        return not tex.get("uvw")
    return any(key.endswith("_map") for key in (side.get("pbr") or {}))


def explode_geometry(m, src, xform, inverse, invertible, paint):
    """``ExplodeGroupCommand``'s geometry loop as one store op
    (``MeshStore.paste_from``): ``src``'s faces and edges into ``m``
    through ``xform`` (None: as they are), each new face wearing what it
    showed inside the group. Returns ``(added faces, change, keys)``,
    ``keys[kind]`` the ``(source key, key in m)`` pairs (vertices and
    edges: the one now at the source element's mapped place); None when
    the store cannot do it (the Python loop runs)."""
    from core.group import placed_face_paint
    from core.materials import effective_attrs
    from core.store_mesh import FaceView, _hole_filter
    if _store_mesh(m) is None or _store_mesh(src) is None:
        return None
    st = m._store
    if not hasattr(st, "paste_from"):
        return None
    sst = src._store
    snap = sst.snapshot()
    pos = sst.positions()
    if xform is None:
        mapped = np.ascontiguousarray(pos, dtype=np.float64).reshape(-1, 3)
    else:
        w = xform.map
        mapped = np.array([w(QVector3D(x, y, z)).toTuple()
                           for x, y, z in pos.tolist()],
                          dtype=np.float64).reshape(-1, 3)
    sd = src.__dict__
    src_table, src_own = sd["_attr_table"], sd["_attr_own"]
    fkeys = sst.keys("face")
    rows = sst.arrays()["face_attr"] if fkeys else []
    d = m.__dict__
    # Copy on write: snapshots and deep copies share the table list.
    table = list(d["_attr_table"])
    index = {repr(a): i for i, a in enumerate(table)}

    def value_of(drawn):
        if not drawn:
            return 0
        drawn = {key: (dict(val) if isinstance(val, dict) else val)
                 for key, val in drawn.items()}
        key = repr(drawn)
        i = index.get(key)
        if i is None:
            i = index[key] = len(table)
            table.append(drawn)
        return i + 1

    # What a face showed inside is what it keeps outside: its own paint,
    # or the group's on both sides, pinned to the prototype. Once per
    # attribute row, except where the placing needs the face's normal
    # and for faces with a dict of their own.
    rows = np.asarray(rows, dtype=np.int64)
    face_attr = np.zeros(len(fkeys), dtype=np.uint32)
    single = []                      # (index, attrs) placed face by face
    if src_own:
        own_at = [i for i, k in enumerate(fkeys) if k in src_own]
        single = [(i, src_own[fkeys[i]]) for i in own_at]
        shared = np.ones(len(fkeys), dtype=bool)
        shared[own_at] = False
    else:
        shared = None
    pick = np.arange(len(rows)) if shared is None else np.flatnonzero(shared)
    order = pick[np.argsort(rows[pick], kind="stable")]
    uniq, starts = np.unique(rows[order], return_index=True)
    bounds = list(starts.tolist()) + [len(order)]
    for n, r in enumerate(uniq.tolist()):
        at = order[bounds[n]:bounds[n + 1]]
        base = src_table[r] if r >= 0 else None
        drawn = effective_attrs(dict(base or {}), paint)
        if invertible and (_needs_face(drawn) or _needs_face(drawn.get("back"))):
            single.extend((i, base) for i in at.tolist())
            continue
        if invertible:
            drawn = placed_face_paint(drawn, FaceView(src, fkeys[int(at[0])]),
                                      inverse)
        face_attr[at] = value_of(drawn)
    for i, base in sorted(single, key=lambda x: x[0]):
        drawn = effective_attrs(dict(base or {}), paint)
        if invertible:
            drawn = placed_face_paint(drawn, FaceView(src, fkeys[i]), inverse)
        face_attr[i] = value_of(drawn)
    face_attr = face_attr.tolist()
    (made, vmap, emap), change = st.paste_from(snap, mapped, face_attr,
                                               _hole_filter)
    d["_attr_table"] = table
    m._dirty(1)
    from core.store_mesh import _Indexed
    added = _Indexed(m, made, FaceView)
    keys = CopiedKeys({"vertex": (sst.keys("vertex"), vmap),
                       "edge": (sst.keys("edge"), emap),
                       "face": (fkeys, made)})
    return added, change, keys


class CopiedKeys(Mapping):
    """``{kind: [(source key, key)]}``, the pairs built when first read
    (a 100k-face explode makes 400k of them)."""

    def __init__(self, columns, alive=None):
        self._columns = columns
        self._alive = alive
        self._pairs: dict = {}

    def only_live(self, store) -> "CopiedKeys":
        """The same map without the keys ``store`` no longer has."""
        return CopiedKeys(self._columns, store.is_live)

    def __getitem__(self, kind):
        got = self._pairs.get(kind)
        if got is None:
            src, dst = self._columns[kind]
            alive = self._alive
            got = self._pairs[kind] = [
                (a, b) for a, b in zip(src, dst)
                if b is not None and (alive is None or alive(b))]
        return got

    def __iter__(self):
        return iter(self._columns)

    def __len__(self):
        return len(self._columns)


def merge_candidates(mesh, added, tol):
    """The broad phase of ``_merge_overlapping_planes`` when its ``before``
    is every face the mesh had besides ``added``: ``(new, pairs)`` with
    ``new`` the added faces with a normal and ``pairs`` ``[(old face,
    [indices into new])]`` in Python's order; None to run the Python scan."""
    m = _store_mesh(mesh)
    h = _hypot()
    if m is None or h is None or not hasattr(m._store, "plane_merge_candidates"):
        return None
    from core.store_mesh import FaceView, _Indexed
    keys = getattr(added, "_keys", None)
    if keys is None:
        keys = [f.key for f in added]
    new, pairs = m._store.plane_merge_candidates(list(keys), tol, h)
    return (_Indexed(m, new, FaceView),
            [(FaceView(m, k), hits) for k, hits in pairs])


# ---------------------------------------------------------------- purge

def material_names(mesh):
    """``{f.attrs.get("mat") for f in mesh.faces}`` minus empties, from the
    attribute rows the faces use and the faces' own dicts — no face gets a
    dict of its own for the question."""
    m = _store_mesh(mesh)
    if m is None:
        return None
    d = m.__dict__
    table, own = d["_attr_table"], d["_attr_own"]
    st = m._store
    fa = st.arrays()["face_attr"]
    if own:
        keys = st.keys("face")
        mask = np.array([k not in own for k in keys], dtype=bool)
        rows = fa[mask] if len(fa) else fa
        mats = {a.get("mat") for k, a in own.items() if st.is_live(k)}
    else:
        rows, mats = fa, set()
    for r in np.unique(rows).tolist():
        if r >= 0:
            mats.add((table[r] or {}).get("mat"))
    mats.discard(None)
    mats.discard("")
    return mats


def layer_tally(mesh, tally) -> bool:
    """Add every face's and edge's tag (``layer_of``) to ``tally`` (a
    ``Counter``; None counts the default tag) from the arrays; False when
    ``mesh`` is not store-backed."""
    m = _store_mesh(mesh)
    if m is None:
        return False
    d = m.__dict__
    table, own = d["_attr_table"], d["_attr_own"]
    st = m._store
    a = st.arrays()
    fa = a["face_attr"]
    if own:
        keys = st.keys("face")
        mask = np.array([k not in own for k in keys], dtype=bool)
        rows = fa[mask] if len(fa) else fa
        for k, attrs in own.items():
            if st.is_live(k):
                tally[(attrs or {}).get("layer") or None] += 1
    else:
        rows = fa
    if len(rows):
        uniq, counts = np.unique(rows, return_counts=True)
        for r, n in zip(uniq.tolist(), counts.tolist()):
            name = (table[r] or {}).get("layer") if r >= 0 else None
            tally[name or None] += int(n)
    layer = a["layer"]
    if len(layer):
        names = list(st.layer_names())
        uniq, counts = np.unique(layer, return_counts=True)
        for i, n in zip(uniq.tolist(), counts.tolist()):
            name = names[i] if 0 <= i < len(names) else None
            tally[name or None] += int(n)
    return True


_ORIGINS = weakref.WeakKeyDictionary()
