# SPDX-License-Identifier: GPL-3.0-or-later
"""Set-compatible element handles indexed by stable identity.

Membership is by identity: mesh elements by their key, so a handle made
again for the same element is the same member; other scene entities
(placements, guides, dimensions…) by Python identity. Iteration yields the
handles in the order they came.

The storage is native (``ingetrazo_core.SelectionStore``,
``rust/ingetrazo_core/src/selection.rs``), so readers on the frame path get
the selection's shape without walking Python handles:

- ``snapshot()``: ``(count, placements, faces, edges, vertices, others,
  revision)``;
- ``revision``: a process-wide counter taken on every change of content,
  and when a set is made — a scene whose selection object is replaced
  reads as changed too;
- ``placement_uids()``, ``element_keys("face" | "edge" | "vertex")`` and
  ``ends()`` (the first and last handle).
"""
from collections.abc import MutableSet

from ingetrazo_core import SelectionStore


class ElementSet(MutableSet):

    def __init__(self, values=()):
        self._store = SelectionStore(values)

    def __deepcopy__(self, memo):
        from copy import deepcopy
        copied = type(self)(deepcopy(list(self), memo))
        memo[id(self)] = copied
        return copied

    def __contains__(self, value):
        return value in self._store

    def __iter__(self):
        return self._store.values()

    def __len__(self):
        return len(self._store)

    def add(self, value):
        self._store.add(value)

    def discard(self, value):
        self._store.discard(value)

    def clear(self):
        self._store.clear()

    def copy(self):
        return type(self)(self)

    def update(self, *others):
        for other in others:
            for value in other:
                self._store.add(value)

    def difference_update(self, *others):
        for other in others:
            for value in list(other):
                self._store.discard(value)

    def intersection_update(self, *others):
        for other in others:
            self._store.retain(list(other))

    def union(self, *others):
        result = self.copy()
        result.update(*others)
        return result

    def difference(self, *others):
        result = self.copy()
        result.difference_update(*others)
        return result

    def intersection(self, *others):
        result = self.copy()
        result.intersection_update(*others)
        return result

    def symmetric_difference(self, other):
        return self ^ type(self)(other)

    def symmetric_difference_update(self, other):
        self._store.replace(self.symmetric_difference(other)._store)

    def issubset(self, other):
        return self <= type(self)(other)

    def issuperset(self, other):
        return self >= type(self)(other)

    # ---- The native snapshot ---------------------------------------------

    @property
    def revision(self) -> int:
        return self._store.revision

    def snapshot(self) -> tuple:
        """``(count, placements, faces, edges, vertices, others, revision)``."""
        return self._store.snapshot()

    def placement_uids(self) -> list:
        return self._store.placement_uids()

    def element_keys(self, kind: str) -> list:
        return self._store.element_keys(kind)

    def ends(self) -> tuple:
        """The first and the last handle, or ``(None, None)``."""
        return self._store.ends()
