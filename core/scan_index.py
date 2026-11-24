# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Native prefilters for the drawing commands' searches over a whole mesh.

Drawing one line across a face of the ride's largest mesh (266,960 faces)
took 20 s on Python objects and 25 s on the store, and erasing an edge or
deleting a face 5–7 s: not the mesh operations, but the commands asking
every face and edge a question almost all of them answer no to (is your
box near the segment, is this loop inside you, do you lie on this plane,
does an edge end at this point). :class:`Scan` puts the mesh, at one
revision, into flat arrays (``ingetrazo_core.MeshScan``: straight off the
store's columns, or read once from a Python mesh's lists) and answers those
questions there.

Every query is a PREFILTER: it returns, in list order, a superset of the
elements the command's own predicate can accept — its tolerances widened,
never narrowed — and the command runs that predicate, unchanged, on what
comes back. The result is the full scan's, element for element. Callers
keep their original loop for when :func:`scan` returns ``None`` (no engine,
or ``INGETRAZO_SCANS=0``, which the differential tests use as the oracle).

A scan is cached per mesh and revision: the planner asks dozens of
questions of one unchanged mesh, and a command that edits it gets a fresh
scan at its next question (a store scan costs a few milliseconds; a Python
mesh's is read from its objects, ~0.1–0.2 s on that mesh).
"""
from __future__ import annotations

import os
import weakref
from collections import OrderedDict

import ingetrazo_core as kernel
from ingetrazo_core import Vec3 as QVector3D

#: ``id(mesh) -> (weakref to it, token, Scan)``, most recent last. A few
#: meshes only: a scan holds a Python mesh's element lists (or a store
#: state's chunks) as they were, and the meshes being drawn in are few.
_CACHE: "OrderedDict" = OrderedDict()
_CACHE_SIZE = 4

#: Tests flip this to run the original full scans (the oracle).
_forced_off = False


def enabled() -> bool:
    """Whether scans run natively: the engine has ``MeshScan`` and nothing
    turned them off (``INGETRAZO_SCANS=0``)."""
    if _forced_off or os.environ.get("INGETRAZO_SCANS") == "0":
        return False
    from core import _native
    mod = _native.native
    return mod is not None and hasattr(mod, "MeshScan")


def _token(mesh):
    from core import store_mesh
    if type(mesh) is store_mesh.StoreMesh:
        st = mesh.__dict__["_store"]
        return (store_mesh.StoreMesh, id(st), st.lineage, st.revision)
    return (type(mesh), mesh.revision, len(mesh.vertices), len(mesh.edges),
            len(mesh.faces))


def store_backed(mesh) -> bool:
    """Whether ``mesh`` lives in the Rust store (its scans read the store's
    columns, and its revision moves with every change)."""
    from core import store_mesh
    return type(mesh) is store_mesh.StoreMesh


def scan(mesh, fresh: bool = False) -> "Scan | None":
    """The mesh's :class:`Scan` at its current revision, or ``None`` when
    scans are off (the caller then runs its own loop). ``fresh``: build a
    new one even when the cache holds one for this revision — a Python
    mesh's revision does not move for every change (a face turned over in
    place, an ``interior`` mark), and a scan reads its objects lazily."""
    if mesh is None or not enabled():
        return None
    from core import mesh as _mesh
    if type(mesh) is _mesh.DeferredMesh:
        len(mesh.faces)                 # built now, as the loop would have
    token = _token(mesh)
    k = id(mesh)
    hit = _CACHE.get(k)
    if (not fresh and hit is not None and hit[0]() is mesh
            and hit[1] == token):
        _CACHE.move_to_end(k)
        return hit[2]
    s = Scan(mesh)
    _CACHE[k] = (weakref.ref(mesh), token, s)
    _CACHE.move_to_end(k)
    while len(_CACHE) > _CACHE_SIZE:
        _CACHE.popitem(last=False)
    return s


def forget(mesh=None) -> None:
    """Drop the cached scan of ``mesh`` (every one without a mesh)."""
    if mesh is None:
        _CACHE.clear()
    else:
        _CACHE.pop(id(mesh), None)


raw_box = kernel.push_queries.raw_box
box_of = kernel.push_queries.box_of


def _xyz(v: QVector3D):
    return (v.x(), v.y(), v.z())


class Scan:
    """One mesh at one revision (see the module docstring). Indices are
    list positions AT THAT REVISION; the element methods turn them into the
    elements the mesh's lists held then, so a command may keep using them
    after it starts editing.

    A Python mesh's scan reads each part (positions, edge ends, edge
    flags, face loops) from the objects at its first query, so a command
    that keeps a scan across its own edits asks everything it will need
    from it before the first edit (``history._SegmentIndex`` does)."""

    @classmethod
    def of_faces(cls, mod, faces) -> "Scan":
        """A scan of some Python faces alone (no vertex or edge list):
        face queries over them, indices into ``faces``."""
        self = cls.__new__(cls)
        self._mesh = lambda: None
        self.store_backed = False
        self.native = mod.MeshScan.from_objects([], [], list(faces))
        self._vec = {}
        self._tkey = {}
        return self

    def __init__(self, mesh) -> None:
        from core import store_mesh
        self._mesh = weakref.ref(mesh)
        self.store_backed = type(mesh) is store_mesh.StoreMesh
        if self.store_backed:
            self.native = kernel.MeshScan.from_store(mesh.__dict__["_store"])
        else:
            self.native = kernel.MeshScan.from_objects(
                list(mesh.vertices), list(mesh.edges), list(mesh.faces))
        self._vec: dict = {}
        self._tkey: dict = {}

    # ---- elements ------------------------------------------------------------
    def _elements(self, kind: str, idx) -> list:
        idx = list(idx)
        if not idx:
            return []
        got = self.native.keys_at(kind, idx)
        if not self.store_backed:
            return got
        from core import store_mesh
        m = self._mesh()
        cls = {"face": store_mesh.FaceView, "edge": store_mesh.EdgeView,
               "vertex": store_mesh.VertexView}[kind]
        return [cls(m, k) for k in got]

    def faces(self, idx) -> list:
        return self._elements("face", idx)

    def vertices(self, idx) -> list:
        """Listed vertices at ``idx`` (indices below the listed count)."""
        return self._elements("vertex", idx)

    def face_indices(self, faces) -> list:
        """Each face's index in the scan's face list (``None``: not
        there)."""
        return kernel.scan_face_indices(self.native, list(faces))

    def edges(self, idx) -> list:
        return self._elements("edge", idx)

    # ---- box queries (indices ascending = list order) ----------------------
    def faces_meeting(self, box) -> list:
        lo, hi = box
        return self.native.faces_meeting(lo, hi)

    def faces_holding(self, box, pad: float) -> list:
        lo, hi = box
        return self.native.faces_holding(lo, hi, pad)

    def faces_within(self, box) -> list:
        lo, hi = box
        return self.native.faces_within(lo, hi)

    def edges_meeting(self, box) -> list:
        lo, hi = box
        return self.native.edges_meeting(lo, hi)

    # ---- vertices of the scan ------------------------------------------------
    def vec(self, v: int) -> QVector3D:
        """Scan vertex ``v`` as a ``QVector3D`` (the same float32 value the
        element's position holds)."""
        hit = self._vec.get(v)
        if hit is None:
            hit = self._vec[v] = QVector3D(*self.native.position(v))
        return hit

    def tkey(self, v: int) -> tuple:
        """``core.topology._key`` of scan vertex ``v``."""
        hit = self._tkey.get(v)
        if hit is None:
            from core.topology import _key
            hit = self._tkey[v] = _key(self.vec(v))
        return hit

    def edges_ending_at(self, key: tuple) -> list:
        """``(edge, end_a, end_b)`` for the edges with an end whose topology
        key may be ``key`` (a superset: compare :meth:`tkey`), ascending."""
        cell = (round(key[0] * 1e4), round(key[1] * 1e4), round(key[2] * 1e4))
        return self.native.edges_at_cell(cell)
