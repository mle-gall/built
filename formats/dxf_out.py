# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Minimal DXF writer for composer view exports (C4).

Writes an ASCII DXF R12 with LINE entities (and SOLID ones for filled
areas) — the least common denominator
every CAD opens, IngeCAD included (the bridge the ecosystem plan calls
for: IngeTrazo produces the 2D view, IngeCAD signs and prints it).
Coordinates go out in MODEL units (metres): the drawing is the true-size
orthographic view, not paper.

The writing is Rust's (ingetrazo_core sheets::dxf, byte for byte what
this module wrote); this keeps the call for its callers.
"""
from __future__ import annotations

from pathlib import Path


def save_dxf_lines(path: str | Path, segments, layer: str = "VISTA") -> int:
    """Write ``segments`` — an iterable of (x0, y0, x1, y1) in metres —
    as LINE entities on ``layer``. Returns the number of lines written."""
    return save_dxf_layers(path, [(layer, segments)])


def save_dxf_layers(path: str | Path, groups, solids=(), texts=()) -> int:
    """Write several ``(layer, segments)`` groups into one DXF — the
    composer's line classes (edges / profiles / section cut) each on its
    own layer, so the CAD's pen table gives them their weights. Empty
    groups still declare their layer. Returns the number of lines written.

    R12 ASCII needs no HEADER section; readers default sensibly, and
    omitting it sidesteps every version-variable there is.

    A group may carry a third element, its linetype: ``"DASHED"`` for the
    hidden lines (issue #81), ``"DASHDOT"`` or ``"DOT"`` for a model layer
    inked that way — declared in an LTYPE table so any CAD draws that layer
    so on its own. A fourth, an RGB colour (0–1), colours the layer with
    the nearest colour of the standard palette (R12 has no true colour). A
    fifth, a dash pattern in drawing units, declares a line type of its own
    by the third's name (a document's custom line type).

    ``solids`` — ``(layer, quads, rgb)`` groups, each quad four corners
    (bottom left, bottom right, top left, top right) — go out as SOLID
    entities on their layer, first, so the lines are drawn over them: the
    filled cuts and the grounds under the hatches. ``texts`` — ``(layer, x,
    y, height, degrees, text)`` — go out as TEXT entities centred on their
    point: the words a line type writes along its lines."""
    import ingetrazo_core as NATIVE
    return NATIVE.sheet_dxf(str(path), list(groups), list(solids), list(texts))
