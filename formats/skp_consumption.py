# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Last-use bookkeeping for an adapter-owned OpenSKP model's geometry."""
from collections import OrderedDict

from ingetrazo_core import FlatGeometry, PrototypeGeometry, image_stage_geometry
from contextvars import ContextVar

_geometry_cache = ContextVar("skp_geometry_cache", default=None)


class _GeometryCache:
    """Bound adapter-owned derived geometry for one adaptation.

    Eviction drops only our cached reference; callers currently reading the
    geometry keep their local references. Raw definition geometry is untouched.
    """

    def __init__(self, limit):
        self.limit = limit
        self.entries = OrderedDict()
        self.size = 0

    def remember(self, definition, attribute, value, size):
        key = (id(definition), attribute)
        previous = self.entries.pop(key, None)
        if previous is not None:
            self.size -= previous[3]
        self.entries[key] = (definition, attribute, value, size)
        self.size += size
        while self.size > self.limit:
            self._evict()

    def _evict(self):
        _key, (definition, attribute, value, size) = self.entries.popitem(last=False)
        self.size -= size
        if getattr(definition, attribute, None) is value:
            delattr(definition, attribute)

    def forget(self, definition, attribute):
        entry = self.entries.pop((id(definition), attribute), None)
        if entry is not None:
            self.size -= entry[3]

    def clear(self):
        while self.entries:
            self._evict()


def _remember_geometry(definition, attribute, value, size):
    try:
        setattr(definition, attribute, value)
    except AttributeError:  # __slots__ definition: skip the cache
        return
    cache = _geometry_cache.get()
    if cache is not None:
        cache.remember(definition, attribute, value, size)


class SoftSegments:
    """Ordered endpoint pairs backed by packed model coordinates.

    Sharing the array avoids six Python floats and three tuples per soft edge. Iteration and repr
    preserve the former tuple/list representation used by prototype deduplication.
    """

    def __init__(self):
        self.chunks = []
        self.size = 0

    def add(self, points):
        points = points.reshape(-1, 2, 3)
        points.setflags(write=False)
        self.chunks.append(points)
        self.size += len(points)

    def __len__(self):
        return self.size

    def __iter__(self):
        for chunk in self.chunks:
            for pair in chunk:
                yield tuple(pair[0].tolist()), tuple(pair[1].tolist())

    def __getitem__(self, index):
        if isinstance(index, slice):
            return [self[i] for i in range(self.size)[index]]
        if index < 0:
            index += self.size
        if index < 0 or index >= self.size:
            raise IndexError(index)
        for chunk in self.chunks:
            if index < len(chunk):
                pair = chunk[index]
                return tuple(pair[0].tolist()), tuple(pair[1].tolist())
            index -= len(chunk)
        raise IndexError(index)

    def __repr__(self):
        return "[" + ", ".join(map(repr, self)) + "]"

    def __eq__(self, other):
        if isinstance(other, (list, SoftSegments)):
            return list(self) == list(other)
        return NotImplemented
