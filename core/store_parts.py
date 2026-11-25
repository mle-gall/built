# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Lazy element sequences and copy provenance for native store operations."""
from __future__ import annotations

import weakref
from collections.abc import Mapping, Sequence

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


_ORIGINS = weakref.WeakKeyDictionary()


def origin_of(mesh):
    """``(source mesh, CopiedKeys)`` for a mesh :func:`copy_part` made from
    a store-backed mesh, else ``(None, None)``."""
    got = _ORIGINS.get(mesh)
    if got is None:
        return None, None
    src = got[0]()
    return (src, got[1]) if src is not None else (None, None)


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
