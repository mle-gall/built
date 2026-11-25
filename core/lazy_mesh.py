# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A deferred mesh's shape as arrays: what building it WOULD make, without
making a single Python object (the experimental engine's lazy meshes).

:func:`plan` replays :meth:`Mesh.bulk_weld`, :meth:`Mesh.add_edges_welded`
and :meth:`Mesh.add_faces_welded` on an empty mesh in NumPy: the vertex
positions in ``mesh.vertices`` order, the edges in ``mesh.edges`` order
with their flags, the faces' corners as vertex indices and each edge's
incident faces in ``edge.faces`` order. tests/test_lazy_mesh_plan.py holds
it to the built mesh, index for index."""
from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np

from core import mesh as _mesh


@dataclass
class Plan:
    pos: np.ndarray            # (V, 3) float64, in source weld precision
    edges: np.ndarray          # (E, 2) int64 vertex indices, v0 → v1
    soft: np.ndarray           # (E,) bool
    hidden: np.ndarray         # (E,) bool
    curve: list                # (E,) int or None
    layer: list                # (E,) str or None
    corners: np.ndarray        # every ring corner, vertex index
    ring_sizes: np.ndarray
    ring_counts: np.ndarray    # rings per face
    ef_start: np.ndarray       # (E + 1,) CSR into ef_face
    ef_face: np.ndarray        # incident face index, edge.faces order
    extra: dict = field(default_factory=dict)
    #: (E,) bool: soft edges the shading does not smooth across
    #: (``Edge.flat``); None for a plan made before the flag (none are).
    flat: np.ndarray | None = None

    @property
    def smooth(self) -> np.ndarray:
        """(E,) bool: the edges the shading runs smooth across."""
        return self.soft if self.flat is None else self.soft & ~self.flat

    @property
    def nfaces(self) -> int:
        return len(self.ring_counts)


def weld(pos: np.ndarray, sequential: bool = False, *, precise=None):
    """``(vpos, point_ids)``: what :meth:`Mesh.bulk_weld` returns on an EMPTY
    mesh, as positions instead of vertices (same order: the vertex list).
    ``sequential``: in the order :meth:`Mesh.vertex` called point by point
    makes them instead — the same vertices, each where its point comes.
    The source array determines the default precision for both mesh backings.
    A format with its own precision may select it explicitly."""
    if precise is None:
        precise = np.asarray(pos).dtype != np.float32
    vpos, ids, origin = _weld(pos, precise=precise)
    if sequential and len(vpos):
        order = np.argsort(origin, kind="stable")
        rank = np.empty_like(order)
        rank[order] = np.arange(len(order))
        vpos, ids = vpos[order], rank[ids]
    return vpos, ids


def _weld(pos: np.ndarray, *, precise=False):
    """``(vpos, point_ids, origin)`` — the weld, and the input point each
    vertex was made at."""
    from ingetrazo_core import store_weld_indices, weld_points
    pos = np.ascontiguousarray(pos, dtype=np.float64 if precise else np.float32)
    if not len(pos):
        return np.zeros((0, 3)), np.empty(0, np.int64), np.empty(0, np.int64)
    tol2 = (10.0 ** -_mesh._KEY_DECIMALS) ** 2
    if precise:
        ids, origin = store_weld_indices(pos, _mesh._KEY_SCALE, tol2)
        return pos[origin], ids, origin
    vpos, ids, origin = weld_points(pos, _mesh._KEY_SCALE, tol2)
    return vpos.astype(np.float64), ids, origin


def plan(pos, edges, flags, curves, layers, layer_names, rings, sizes,
         counts, sequential=False) -> Plan:
    """The built mesh of a format-3 block (already cleared of non-finite
    points), as :class:`Plan`. ``sequential``: built by the per-entity
    walk (vertices in the order each one's point comes)."""
    from ingetrazo_core import deferred_incidence

    def column(values):
        return None if values is None else np.ascontiguousarray(values, dtype=np.int64)

    vpos, inverse = weld(pos, sequential)
    sizes, counts = column(sizes), column(counts)
    data = deferred_incidence(inverse, column(edges).reshape(-1, 2), column(flags),
                              column(curves), column(layers), layer_names,
                              column(rings), sizes, counts)
    return Plan(pos=vpos, ring_sizes=sizes, ring_counts=counts, **data)


def plan_of(mesh) -> Plan | None:
    """The :class:`Plan` of a still-deferred mesh (memoised on it), or
    ``None`` once it is built or when it was never deferred."""
    if type(mesh) is not _mesh.DeferredMesh:
        return None
    d = mesh.__dict__
    hit = d.get("_deferred_plan")
    if hit is None:
        s = d["_deferred_source"]
        if s is None:  # A custom deferred builder need not supply arrays.
            return None
        hit = d["_deferred_plan"] = plan(
            s["pos"], s["edges"], s["flags"], s["curves"], s["layers"],
            s["meta"].get("edge_layers") or [], s["rings"], s["sizes"],
            s["counts"], s.get("sequential", False))
    return hit


def face_net_areas(plan) -> np.ndarray:
    """Face.net_area over planned vertices, including holes, without topology.

    Accumulate each ring in corner order in doubles, then use Qt's float
    vector length just as Face does. All rings advance together in NumPy.
    """
    from ingetrazo_core import Vec3 as QVector3D
    sizes = plan.ring_sizes
    if not len(sizes):
        return np.zeros(plan.nfaces)
    starts = np.concatenate([[0], np.cumsum(sizes[:-1])])
    normal = np.zeros((len(sizes), 3), dtype=np.float64)
    for i in range(int(sizes.max(initial=0))):
        active = np.flatnonzero((sizes > i) & (sizes >= 3))
        a = plan.pos[plan.corners[starts[active] + i]]
        b = plan.pos[plan.corners[starts[active] + (i + 1) % sizes[active]]]
        normal[active, 0] += (a[:, 1] - b[:, 1]) * (a[:, 2] + b[:, 2])
        normal[active, 1] += (a[:, 2] - b[:, 2]) * (a[:, 0] + b[:, 0])
        normal[active, 2] += (a[:, 0] - b[:, 0]) * (a[:, 1] + b[:, 1])
    areas = np.fromiter((0.5 * QVector3D(*map(float, n)).length()
                         for n in normal), dtype=np.float64, count=len(sizes))
    counts = plan.ring_counts
    first = np.concatenate([[0], np.cumsum(counts[:-1])])
    result = areas[first].copy()
    for i in range(1, int(counts.max(initial=0))):
        active = np.flatnonzero(counts > i)
        result[active] -= areas[first[active] + i]
    result[sizes[first] < 3] = 0
    return np.maximum(result, 0)


def entity_count(mesh) -> int:
    """``len(mesh.faces) + len(mesh.edges)``, a lazy mesh left unbuilt."""
    plan = plan_of(mesh)
    if plan is not None:
        return plan.nfaces + len(plan.edges)
    return len(mesh.faces) + len(mesh.edges)


class PlannedVertices:
    """``mesh.vertices`` of a lazy mesh for code that only reads their
    positions: ``pv[i].position`` is the i-th planned vertex's position (a
    fresh QVector3D), ``len(pv)`` their count — nothing built."""

    __slots__ = ("pos",)

    class _V:
        __slots__ = ("position",)

        def __init__(self, p):
            from ingetrazo_core import Vec3 as QVector3D
            self.position = QVector3D(float(p[0]), float(p[1]), float(p[2]))

    def __init__(self, pos):
        self.pos = pos

    def __len__(self):
        return len(self.pos)

    def __getitem__(self, i):
        return PlannedVertices._V(self.pos[int(i)])
