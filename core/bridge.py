# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Bridge (issue #614): side faces between two profiles.

A profile is a face (its outline, and its holes), a closed loop of edges or
an open chain of them. Two selected profiles are joined:

* face to face — the faces stay as the two caps, so two loose rectangles
  become a closed box;
* face to line — the line is walked there and back as a flat loop, so a
  rectangle and a ridge line become a hip roof;
* line to line — a ruled surface between the two chains.

Points are paired one to one when both profiles have as many; otherwise by
the fraction of the length walked along each, which gives quads where the
two meet and triangles in between. Closed profiles are first turned and, if
need be, reversed so the bridge does not twist. A side that is not flat is
split along its shorter diagonal, softened so it still reads as one surface.

Headless: :func:`profiles_from_selection` reads the selection and
:func:`bridge` builds the faces; the menu command is a thin shell over them.
"""
from __future__ import annotations

from ingetrazo_core import (
    align_chain, align_loop, bridge, bridge_plan as plan, follow_neighbours,
    profiles_from_selection, strips,
)


class BridgeError(ValueError):
    """The selection cannot be bridged; the message says why (English
    source text, translated by the caller)."""


class Profile:
    """One side of a bridge: ``loops`` of points (a face's outline then its
    holes, or a single chain), ``closed`` or not, and the ``face`` it came
    from (kept as a cap) when it is one."""

    def __init__(self, loops, closed: bool, face=None) -> None:
        self.loops = [list(lp) for lp in loops]
        self.closed = closed
        self.face = face

    @property
    def points(self) -> list:
        return [p for lp in self.loops for p in lp]
