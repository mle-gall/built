# SPDX-License-Identifier: GPL-3.0-or-later
"""Set-compatible element handles indexed by stable identity.

Iteration still yields Python handles during the Python kernel transition;
identity and membership are keys, including when a handle is reconstructed.
Non-mesh scene entities retain their Python identity.
"""
from collections.abc import MutableSet

from core.mesh import element_identity


class ElementSet(MutableSet):
    def __init__(self, values=()):
        self._items = {element_identity(v): v for v in values}

    def __deepcopy__(self, memo):
        from copy import deepcopy
        copied = type(self)(deepcopy(list(self), memo))
        memo[id(self)] = copied
        return copied

    def __contains__(self, value):
        return element_identity(value) in self._items

    def __iter__(self):
        return iter(self._items.values())

    def __len__(self):
        return len(self._items)

    def add(self, value):
        self._items[element_identity(value)] = value

    def discard(self, value):
        self._items.pop(element_identity(value), None)

    def clear(self):
        self._items.clear()

    def copy(self):
        return type(self)(self)

    def update(self, *others):
        for other in others:
            for value in other:
                self.add(value)

    def difference_update(self, *others):
        for other in others:
            for value in list(other):
                self.discard(value)

    def intersection_update(self, *others):
        for other in others:
            keep = {element_identity(v) for v in other}
            self._items = {k: v for k, v in self._items.items() if k in keep}

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
        self._items = self.symmetric_difference(other)._items

    def issubset(self, other):
        return self <= type(self)(other)

    def issuperset(self, other):
        return self >= type(self)(other)
