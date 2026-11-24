# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Last-use bookkeeping for an adapter-owned OpenSKP model's geometry."""
from functools import partialmethod
from ingetrazo_core import (
    GeometryCache as _GeometryCache, skp_remember as _remember_geometry,
    skp_segments_add, skp_segments_iter, skp_segments_get, skp_packed_eq, skp_packed_repr,
)

from ingetrazo_core import FlatGeometry, PrototypeGeometry, image_stage_geometry
from contextvars import ContextVar

_geometry_cache = ContextVar("skp_geometry_cache", default=None)


class SoftSegments:
    """Ordered endpoint pairs backed by packed model coordinates.

    Sharing the array avoids six Python floats and three tuples per soft edge. Iteration and repr
    preserve the former tuple/list representation used by prototype deduplication.
    """

    def __init__(self):
        self.chunks = []
        self.size = 0

    add = partialmethod(skp_segments_add)

    def __len__(self):
        return self.size

    __iter__ = partialmethod(skp_segments_iter)
    __getitem__ = partialmethod(skp_segments_get)
    __repr__ = partialmethod(skp_packed_repr)
    __eq__ = partialmethod(skp_packed_eq)
