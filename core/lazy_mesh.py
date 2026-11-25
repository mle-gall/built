# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A deferred mesh in native weld, edge and face order, before first access."""
from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np

from ingetrazo_core import deferred_weld as weld
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
    """Net areas of planned faces, including holes, in face order."""
    from ingetrazo_core import planned_face_areas
    return planned_face_areas(plan.pos, plan.corners,
                              np.asarray(plan.ring_sizes, dtype=np.int64),
                              np.asarray(plan.ring_counts, dtype=np.int64))


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
