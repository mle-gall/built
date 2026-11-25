# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A deferred mesh in native weld, edge and face order, before first access."""
from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np

from ingetrazo_core import (PlannedVertices, deferred_plan as plan, deferred_weld as weld,
                            entity_count, plan_of)


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


def face_net_areas(plan) -> np.ndarray:
    """Net areas of planned faces, including holes, in face order."""
    from ingetrazo_core import planned_face_areas
    return planned_face_areas(plan.pos, plan.corners,
                              np.asarray(plan.ring_sizes, dtype=np.int64),
                              np.asarray(plan.ring_counts, dtype=np.int64))
