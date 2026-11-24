# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A model view's lines and section cut grouped as a DXF carries them
(formats.dxf_out.save_dxf_layers writes them): one DXF layer per model
layer, its profiles and cut beside it, the hatched cut as lines and
solid strips. Moved out of views/composer.py unchanged so the Qt host's
view export, the deliveries and the Sheets workspace share it while the
composer goes (docs/rust-sheets.md, eighth slice)."""
from __future__ import annotations

import re

from core.layers import grain_layers, section_cut_kind


#: The scale a view exported without a sheet («File ▸ Export ▸ Current
#: view as DXF») draws its hatches at: the usual one of a plan or a section
#: with its materials.
DXF_VIEW_SCALE = 50.0


def dxf_cut_layers(scene, camera, scale_n: float = DXF_VIEW_SCALE, k=None,
                   frame_scale: float = 1.0, geometry=None) -> tuple:
    """The section cut of ``scene`` under ``camera`` (parallel) as a DXF
    carries it, in MODEL units like the view's lines: ``(groups, solids)``
    for formats.dxf_out.save_dxf_layers. ``groups``: one layer
    «<layer>-TRAMA» per hatched model layer, in its colour (its hatch ink
    when it has one), the pattern as a sheet at 1:``scale_n`` draws it, as
    lines (R12 has no hatch entity: any CAD shows them as they are).
    ``solids``: a layer's solid fill on «<layer>-TRAMA» and the ground under
    its hatch on «<layer>-FONDO», in their colours, as SOLID strips drawn
    under the lines. ``k`` — paper mm per model unit (a frame's; 1000 /
    ``scale_n`` for metres); ``frame_scale`` the frame's own hatch size."""
    import numpy as np
    from core.hatches import fill_quads, hatch_lines
    from core.hlr import hlr_drawing
    from core.layers import (cut_hatch_forms, cut_hatches, hatch_ground_rgb,
                             hatch_ink_rgb, layer_color)
    hatches = cut_hatches(scene)
    if not hatches:
        return [], []
    # ``geometry``: the tagged arrays of Viewport.hlr_geometry(tags=True)
    # (world space, any camera), so the drawing is not walked out of the
    # model.
    drawing = hlr_drawing(scene, camera, profiles=False, fills=True,
                          layers=True, depth_layers=grain_layers(scene),
                          geometry=geometry)
    if drawing.loop_layers is None:
        return [], []
    scale_n = float(scale_n or DXF_VIEW_SCALE)
    k = float(k) if k else 1000.0 / scale_n
    # Paper mm, the paper's y down as on a sheet: the patterns keep the
    # sheet's size, turn and phase.
    forms = cut_hatch_forms(scene, scale_n)
    cut = section_cut_kind(scene)
    by_tag: dict = {}
    depths = drawing.loop_depths or [None] * len(drawing.loops)
    for ring, tag, d in zip(drawing.loops, drawing.loop_layers, depths):
        if tag in hatches:
            rings, ds = by_tag.setdefault(tag, ([], []))
            rings.append([(float(x) * k, -float(y) * k) for x, y in ring])
            ds.append(None if d is None else d * k)

    def back(pts):
        return tuple((x / k, -y / k) for x, y in pts)
    groups, solids = [], []
    # DXF layers go out under the tag's folder path, not its key (as
    # dxf_groups_by_layer): a key may be a uid, and two tags share a label.
    from core.layers import export_labels
    labels = export_labels(scene)
    for tag, (rings, ds) in by_tag.items():
        name = labels.get(tag, tag)
        ly = scene.layer(tag) if hasattr(scene, "layer") else None
        colour = layer_color(ly) if ly is not None else None
        ink = (hatch_ink_rgb(ly) if ly is not None else None) or colour
        ground = hatch_ground_rgb(ly) if ly is not None else None
        quads = None
        if ground is not None or hatches[tag] == "solid":
            # Strips in model units, the right way up again.
            quads = [back((q[2], q[3], q[0], q[1])) for q in fill_quads(rings)]
        if ground is not None:
            solids.append((f"{name}-FONDO", quads, ground))
        if hatches[tag] == "solid":
            solids.append((f"{name}-TRAMA", quads, ink))
            continue
        lines = hatch_lines(hatches[tag], rings,
                            dict(forms.get(tag, {}), cut=cut), frame_scale,
                            ds)
        if lines:
            segs = np.array([(x0 / k, -y0 / k, x1 / k, -y1 / k)
                             for x0, y0, x1, y1 in lines])
            groups.append((f"{name}-TRAMA", segs, "CONTINUOUS", ink))
    return groups, solids


def dxf_groups_by_layer(drawing, view_layer: str, hidden_lines: bool,
                        texts=None) -> list:
    """The DXF groups of a layer-tagged view drawing: one DXF layer per
    model layer — its name, its line type and its colour (the default
    layer keeps the CAD's own) — and its profiles beside it on
    «<layer>-PERFIL», so a plan's outlines stay on their layer and keep
    their own pen; the cut beside them too, on «<layer>-CORTE», continuous
    in the layer's colour (a cut that belongs to no layer stays on the
    view's «-CORTE»); the hidden lines, which belong to no layer, on the
    view's «-OCULTAS». ``texts`` — a list — takes the words of the layers
    whose line type writes some (GAS…), ``(layer, x, y, height, degrees,
    text)`` in drawing units along their lines: R12 has no line type with
    text, so they go as TEXT entities where a sheet at 1:100 puts them."""
    from core.hlr import KIND_CUT, KIND_HIDDEN, KIND_PROFILE
    from core.layers import DXF_LINETYPES
    styles = getattr(drawing, "styles", None) or {}
    # Tags go out under their folder path (core.layers.export_labels), not
    # their key — a key may be a uid, and two tags may share a label.
    labels = getattr(drawing, "labels", None) or {}
    k = drawing.kinds
    tags = drawing.layers
    groups = [(f"{view_layer}-CORTE", drawing.segs[[
        i for i in range(len(k)) if k[i] == KIND_CUT and not tags[i]]])]
    if hidden_lines:
        groups.append((f"{view_layer}-OCULTAS", drawing.segs[k == KIND_HIDDEN],
                       "DASHED"))
    by_tag: dict = {}
    for i in range(len(k)):
        if k[i] == KIND_HIDDEN or (k[i] == KIND_CUT and not tags[i]):
            continue
        suffix = ("-PERFIL" if k[i] == KIND_PROFILE
                  else "-CORTE" if k[i] == KIND_CUT else "")
        by_tag.setdefault((tags[i] or view_layer, suffix), []).append(i)
    for (tag, suffix), ids in by_tag.items():
        style = styles.get(tag) or (None, "continuous")
        rgb, linetype = style[0], style[1]
        label = labels.get(tag, tag) + suffix
        # The sixth element, the layer's own pen in mm, is the SVG
        # export's (R12 DXF leaves weights to the CAD's pen table); its
        # profiles and cuts keep their class's pen.
        weight = style[2] if len(style) > 2 and not suffix else None
        if suffix == "-CORTE":
            linetype = "continuous"
        if linetype in DXF_LINETYPES:
            groups.append((label, drawing.segs[ids],
                           DXF_LINETYPES[linetype], rgb, None, weight))
            continue
        # The document's own line type: declared in the file, by its name,
        # its pattern in drawing units as the standard ones are (paper mm
        # at 1:100 — the CAD's line type scale adapts it).
        pattern = style[4] if len(style) > 4 else ()
        name = re.sub(r"[^A-Za-z0-9_]+", "_",
                      linetype.split(":", 1)[-1]).upper()[:31] or "OWN"
        from core.layers import file_dashes
        groups.append((label, drawing.segs[ids], name, rgb,
                       tuple(v * 0.1 for v in file_dashes(pattern)), weight))
        words = style[5] if len(style) > 5 else None
        if texts is not None and words is not None and suffix != "-CORTE":
            from core.layers import text_marks
            spec = (words[0], words[1] * 0.1, words[2] * 0.1, words[3] * 0.1)
            for x0, y0, x1, y1 in drawing.segs[ids]:
                for x, y, angle in text_marks((float(x0), float(y0)),
                                              (float(x1), float(y1)), spec):
                    texts.append((label, x, y, spec[1], angle,
                                  words[0]))
    return groups
