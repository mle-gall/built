# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Read-only packed coordinate protocol for the internal SKP payload."""

from functools import partialmethod
from ingetrazo_core import (
    skp_rows_init, skp_rows_get, skp_rows_iter, skp_packed_eq, skp_packed_repr,
    skp_flatten_rings as flatten_rings,
)


class PointRows:
    """List-compatible coordinate rows sharing one packed array.

    Faces keep slices of the transformed array instead of allocating a list
    and three Python floats per corner. Reading a row preserves its precision. Repr
    deliberately remains the list repr used by prototype deduplication.
    """

    __slots__ = ("_array", "_indices")

    __init__ = partialmethod(skp_rows_init)

    def _coordinate_span(self):
        """Borrow read-only rows in this exact order; no Python float expansion."""
        return self._array, self._indices

    def __len__(self):
        return len(self._indices)

    __getitem__ = partialmethod(skp_rows_get)
    __iter__ = partialmethod(skp_rows_iter)
    __repr__ = partialmethod(skp_packed_repr)
    __eq__ = partialmethod(skp_packed_eq)
