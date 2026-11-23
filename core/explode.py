# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Exploded view: a component's parts pulled apart, and put back.

Each part moves away from the assembly's centre in proportion to how far it
already sits from it — the classic exploded drawing, where the seat stays
put and the legs spread out around it — either in every direction
(``outward``) or along one axis only (``x``, ``y``, ``z``: a stack pulled
apart vertically reads best along blue).

The parts really move, inside their component, through an undoable command:
so everything that draws or reads the model — the viewport, section cuts,
sheets, snaps, exports — shows the exploded state with no special case.
What makes it reversible is that each part remembers the translation the
explosion gave it (``Group.explode_offset``) and the component remembers
the setting (``Group.exploded``). Changing the amount swaps the old offset
for the new one; Reassemble takes it off. A part moved by hand in between
keeps that move, because only the explosion's own share is ever undone.
"""
from __future__ import annotations

from ingetrazo_core import placement_transform as _transform

MODES = ("outward", "x", "y", "z")


def assembled_centres(container) -> list:
    """Each part's centre (the middle of its box, in the container's frame)
    where it sits assembled — its current centre minus the explosion's
    offset."""
    return _transform.assembled_centres(container)


def explode_offsets(centres, factor: float, mode: str = "outward") -> list:
    """The translation each part takes at ``factor`` (0 = assembled, 1 =
    every part twice as far from the centre as it was). The centre is the
    middle of the parts' centres' box, so a symmetric model explodes
    symmetrically."""
    return _transform.explode_offsets(centres, factor, mode)


def apply_explode(container, factor: float, mode: str = "outward",
                  centres=None) -> None:
    """Pull ``container``'s parts apart to ``factor`` (``0`` reassembles),
    in place. ``centres`` (from :func:`assembled_centres`) lets a slider drag
    measure once and apply many times. The command is
    :class:`ExplodeViewCommand`; this is its body."""
    return _transform.apply_explode(container, factor, mode, centres)


def rotate_offsets(container, xform) -> None:
    """Re-express the parts' offsets after ``xform`` was pushed down into
    them (entering a container for editing bakes its matrix into its
    children, see ``Scene._bake_container_xform``): an offset is a vector
    in the container's frame, so it turns — and scales — with that frame."""
    from ingetrazo_core import Vec3 as QVector3D
    for kid in getattr(container, "children", None) or ():
        o = getattr(kid, "explode_offset", None)
        if not o:
            continue
        v = xform.mapVector(QVector3D(*o))
        kid.explode_offset = (v.x(), v.y(), v.z())


def snapshot(container) -> tuple:
    """Everything an explosion changes, to put back exactly (undo, and the
    slider's live preview)."""
    return _transform.exploded_snapshot(container)


def restore(container, snap) -> None:
    return _transform.restore_exploded(container, snap)
