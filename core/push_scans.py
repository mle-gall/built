# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Native answers to push/pull's searches over a whole mesh.

Pushing a face of the ride's largest mesh (266,960 faces) spent ~40 s in
the click and as much in the first stage. Most of it was orientation
(:mod:`core.orient`, native now); the rest were loops asking every face or
vertex a question only a few near the push can answer yes to: does a face
carry this base edge (``classify_push_edge``), which parallel faces overlap
the base (the inward limit, the stage's levels), where does a ray from the
base leave the solid (the lateral limit), which vertices sit on the cap or
on the base's edges, which faces touch the op's seeds.

Each helper returns either the loop's exact answer or a PREFILTER — the
candidates, in list order, a superset of what the loop's own predicate can
accept, which the caller then runs unchanged — or ``None`` when the scans
are off (no engine, ``INGETRAZO_SCANS=0`` — the oracle the differential
tests compare against), and the caller keeps its loop.

Store-backed meshes are scanned straight off the store's columns, cached
per store revision (every change moves it). A Python mesh's scan reads its
objects; its revision does not move for everything (a vertex placed, a face
turned over in place, an ``interior`` mark), so it is read afresh.
"""
from __future__ import annotations

import sys

from ingetrazo_core import Vec3 as QVector3D


def _scan(mesh):
    from core import scan_index
    if mesh is None:
        return None
    return scan_index.scan(mesh, fresh=not scan_index.store_backed(mesh))


def _mod(name: str):
    from core import _native
    mod = _native.native
    return getattr(mod, name, None) if mod is not None else None


def _geom_args():
    """``(hypot, compensated)`` for the queries that reproduce float32
    geometry, or ``None`` when this Qt's ``length()`` formula is unknown."""
    from core import _native
    h = _native.qt_hypot()
    if h is None:
        return None
    return h, sys.version_info >= (3, 12)


def _xyz(p: QVector3D) -> tuple:
    return (p.x(), p.y(), p.z())


def is_closed(mesh):
    """``core.orient.is_closed(mesh)`` of a store-backed mesh, natively (a
    Python mesh's edge lists are read faster by the loop itself); ``None``
    otherwise."""
    from core import scan_index
    fn = _mod("scan_is_closed")
    if fn is None or not scan_index.store_backed(mesh):
        return None
    s = scan_index.scan(mesh)
    if s is None:
        return None
    return fn(s.native)


def mesh_is_flat(mesh, tol: float):
    """Store verdict, or None to run the object-mesh oracle."""
    from core import _native, scan_index
    if scan_index.store_backed(mesh) and scan_index.enabled():
        hypot = _native.qt_hypot()
        if hypot is not None:
            return mesh._store.faces_are_flat(tol, hypot)
    return None


def faces_on_segment(mesh, a: QVector3D, b: QVector3D):
    """Candidates for ``classify_push_edge(face, a, b, faces)`` and
    ``_segment_on_face_boundary(a, b, g)``: every face with a boundary edge
    (outer or hole) through both points within ``_SPLIT_TOLERANCE``, or
    with corners at their keys, is among them."""
    from core.topology import _SPLIT_TOLERANCE as tol
    fn = _mod("scan_faces_holding_points")
    s = _scan(mesh) if fn is not None else None
    if s is None:
        return None
    # A point within tol of a segment, the segment overrun by tol of its
    # length at either end, or within a key cell of a corner: inside the
    # face's box grown by 2·tol + tol · its diagonal. Doubled.
    idx = fn(s.native, [_xyz(a), _xyz(b)], 4.0 * tol, 2.0 * tol)
    return s.faces(idx)


def parallel_overlapping(mesh, n: QVector3D, base_xy, dot_min: float = 0.999):
    """Candidates for the loops over every face PARALLEL to the unit ``n``
    (``|normal().normalized() · n| ≥ dot_min``) whose loop, projected by
    ``_project_loop_2d(loop, n)``, ``_loops_overlap_2d`` the base's
    ``base_xy``: those faces with the projected boxes meeting."""
    from core.arrangement import plane_basis
    fn = _mod("scan_parallel_overlapping")
    args = _geom_args()
    s = _scan(mesh) if fn is not None and args is not None else None
    if s is None or not base_xy:
        return None
    u, v = plane_basis(n)
    xs = [p[0] for p in base_xy]
    ys = [p[1] for p in base_xy]
    idx = fn(s.native, _xyz(n), _xyz(u), _xyz(v), (min(xs), min(ys)),
             (max(xs), max(ys)), dot_min, *args)
    return s.faces(idx)


def lateral_exit(mesh, base_face, samples, direction: QVector3D,
                 n: QVector3D):
    """``_compute_inward_limit``'s lateral exit, exactly: ``(found, t)`` —
    the least ``t > 1e-4`` at which a ray from one of ``samples`` along
    ``direction`` crosses a triangle of a face other than ``base_face``,
    not interior and not parallel to ``n`` (``t`` None when none does) —
    or ``None`` when the engine cannot answer."""
    fn = _mod("scan_lateral_exit")
    args = _geom_args()
    s = _scan(mesh) if fn is not None and args is not None else None
    if s is None:
        return None
    skip = None
    got = s.face_indices([base_face])
    if got and got[0] is not None:
        skip = got[0]
    t = fn(s.native, [_xyz(q) for q in samples], _xyz(direction), _xyz(n),
           skip, *args)
    return True, t


def inward_limit(mesh, base_face, n: QVector3D, anchor: QVector3D,
                 stitch_tol: float):
    """How far ``base_face`` may be pushed into the solid, natively
    (``pushscan::scan_inward_limit``): the nearest parallel blocker behind
    the whole base, 0 for a backing wall leaning away, the lateral exit a
    hair short; ``None`` unbounded. Always answered (a scan is built even
    with scans off: there is no Python loop behind it any more)."""
    from core import scan_index
    s = _scan(mesh) or scan_index.Scan(mesh)
    args = _geom_args()
    got = s.face_indices([base_face])
    if not got or got[0] is None:
        return None
    return _mod("scan_inward_limit")(s.native, got[0], _xyz(n), _xyz(anchor),
                                     float(stitch_tol), *args)


def vertices_in_boxes(mesh, boxes):
    """The listed vertices inside any of ``boxes`` (``(x0, y0, z0, x1, y1,
    z1)``, bounds included), in list order; ``None`` when scans are off."""
    fn = _mod("scan_vertices_in_boxes")
    s = _scan(mesh) if fn is not None else None
    if s is None:
        return None
    return s.vertices(fn(s.native, [list(b) for b in boxes]))


def vertices_at_keys(mesh, points):
    """Candidates for the vertices whose ``_key`` is one of ``points``'
    keys: those within a key cell (doubled) of one of them."""
    pad = 2e-4
    boxes = []
    for p in points:
        m = pad + 1e-6 * max(abs(p.x()), abs(p.y()), abs(p.z()))
        boxes.append((p.x() - m, p.y() - m, p.z() - m,
                      p.x() + m, p.y() + m, p.z() + m))
    if not boxes:
        return []
    return vertices_in_boxes(mesh, boxes)


def faces_meeting(mesh, lo, hi):
    """Faces whose outer loop's box meets ``[lo, hi]`` (bounds included),
    in list order — exactly the faces a ``max < lo or min > hi`` skip on
    their ``vertices`` keeps; ``None`` when scans are off."""
    s = _scan(mesh)
    if s is None:
        return None
    return s.faces(s.faces_meeting((tuple(lo), tuple(hi))))


def faces_near_points_rings(mesh, points):
    """Candidates for the faces with a corner (outer or hole) whose
    ``_key`` is one of ``points``' keys: faces whose box over every ring
    meets the points' box grown by a key cell (doubled)."""
    fn = _mod("scan_faces_meeting_rings")
    pts = list(points)
    if not pts:
        return []
    s = _scan(mesh) if fn is not None else None
    if s is None:
        return None
    lo, hi = _grown_box(pts)
    return s.faces(fn(s.native, lo, hi))


def edges_near_points(mesh, points):
    """Candidates for the edges with an end whose ``_key`` is one of
    ``points``' keys: edges whose box meets the points' box grown by a key
    cell (doubled), in list order."""
    pts = list(points)
    if not pts:
        return []
    s = _scan(mesh)
    if s is None:
        return None
    return s.edges(s.edges_meeting(_grown_box(pts)))


def _grown_box(pts):
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    zs = [p[2] for p in pts]
    m = 2e-4 + 1e-6 * max(max(map(abs, xs)), max(map(abs, ys)),
                          max(map(abs, zs)))
    return ((min(xs) - m, min(ys) - m, min(zs) - m),
            (max(xs) + m, max(ys) + m, max(zs) + m))


def faces_near_points(mesh, points):
    """Candidates for the faces with an outer corner whose ``_key`` is one
    of ``points``' keys: faces whose outer box meets the points' box grown
    by a key cell (doubled)."""
    pts = list(points)
    if not pts:
        return []
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    zs = [p[2] for p in pts]
    m = 2e-4 + 1e-6 * max(max(map(abs, xs)), max(map(abs, ys)),
                          max(map(abs, zs)))
    return faces_meeting(mesh, (min(xs) - m, min(ys) - m, min(zs) - m),
                         (max(xs) + m, max(ys) + m, max(zs) + m))


def cracked_edges(mesh, near=None):
    """``crack_planes``' edges, exactly: listed edges bordering one face
    with an end inside ``near`` (all of them without), in list order."""
    fn = _mod("scan_cracked_edges")
    s = _scan(mesh) if fn is not None else None
    if s is None:
        return None
    return s.edges(fn(s.native, None if near is None else list(near)))


# ---- element sets by key ----------------------------------------------------

def live_keys(mesh, kind: str):
    """The live keys of ``kind`` ("vertex", "edge", "face") of a
    store-backed mesh, in list order (ints); ``None`` for a Python mesh or
    with the scans off."""
    from core import scan_index
    if not scan_index.enabled() or not scan_index.store_backed(mesh):
        return None
    st = mesh.__dict__["_store"]
    base = st.key_base
    if base < (1 << 64):
        # The same ints, made from a uint64 array (a u128 per element
        # through pyo3 cost ~7 ms per 267k faces).
        import numpy as np
        return (st.local_keys(kind) | np.uint64(base)).tolist()
    return st.keys(kind)


def views(mesh, kind: str, keys) -> list:
    from core import store_mesh
    cls = {"face": store_mesh.FaceView, "edge": store_mesh.EdgeView,
           "vertex": store_mesh.VertexView}[kind]
    return [cls(mesh, k) for k in keys]


def mark(mesh):
    """A mark of a store-backed mesh's current elements (its snapshot,
    O(chunks)) for :func:`made_since`; ``None`` for a Python mesh or with
    the scans off."""
    from core import scan_index
    if (_mod("store_keys_since") is None or not scan_index.enabled()
            or not scan_index.store_backed(mesh)):
        return None
    return mesh.__dict__["_store"].snapshot()


def made_since(mesh, mark, kind: str):
    """The elements of ``kind`` ``mesh`` holds now and did not at ``mark``
    (a store snapshot: :func:`mark`, or a ``StoreState``'s ``snap``), as
    views in list order — ``[x for x in mesh.<kind>s if x not in <then>]``
    — or ``None`` (no mark, or the mesh left the store meanwhile)."""
    from core import scan_index
    fn = _mod("store_keys_since")
    if mark is None or fn is None or not scan_index.store_backed(mesh):
        return None
    return views(mesh, kind, fn(mesh.__dict__["_store"], mark, kind))


def set_minus(mesh, kind: str, after_keys, before: set):
    """``set(<the views of after_keys>) - <a set of views keyed before>``,
    built exactly as CPython builds it, iteration order included: a view
    hashes and compares as its key, so a set of the keys has the same
    table, and the difference walks it in slot order adding what ``before``
    lacks. ``None`` when CPython would take its other path (copy the left
    side, discard the right: a left side four times the right's size)."""
    so = set(after_keys)
    if (len(so) >> 2) > len(before):
        return None
    from core import store_mesh
    cls = {"face": store_mesh.FaceView, "edge": store_mesh.EdgeView,
           "vertex": store_mesh.VertexView}[kind]
    out = set()
    for k in so:
        if k not in before:
            out.add(cls(mesh, k))
    return out


def _native_face(mesh, face):
    """``(scan, index)`` of ``face`` on a scan of ``mesh`` (built even with
    the scans off: these questions have no Python loop behind them)."""
    from core import scan_index
    s = _scan(mesh) or scan_index.Scan(mesh)
    got = s.face_indices([face])
    if not got or got[0] is None:
        return None, None
    return s, got[0]


def classify_base(mesh, face):
    """``(attached, prism_cap)`` of a push's base (``pushscan::scan_classify_base``)."""
    s, i = _native_face(mesh, face)
    if s is None:
        return False, False
    return _mod("scan_classify_base")(s.native, i, *_geom_args())


def sweeps_free_sheet(mesh, face, loops) -> bool:
    """Whether no face off ``face``'s plane stands on an edge of ``loops``
    (``pushscan::scan_sweeps_free_sheet``)."""
    s, i = _native_face(mesh, face)
    if s is None:
        return True
    return _mod("scan_sweeps_free_sheet")(
        s.native, i, [[_xyz(p) for p in lp] for lp in loops], *_geom_args())


def surface_normal(mesh, face) -> QVector3D:
    """``face``'s unit normal turned to agree with its coplanar neighbours
    (``pushscan::scan_surface_normal``)."""
    s, i = _native_face(mesh, face)
    if s is None:
        return face.normal().normalized()
    return QVector3D(*_mod("scan_surface_normal")(s.native, i, *_geom_args()))


def cap_passes_wall(mesh, face, cap, normal: QVector3D, d: float) -> bool:
    """Whether shrinking the prism cap ``cap`` by ``d`` passes a vertex
    partway up a face with a corner on its rim (``pushscan::scan_cap_passes_wall``)."""
    s, i = _native_face(mesh, face)
    if s is None:
        return False
    return _mod("scan_cap_passes_wall")(s.native, i, [_xyz(p) for p in cap], _xyz(normal),
                                        float(d), *_geom_args())
