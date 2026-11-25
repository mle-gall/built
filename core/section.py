# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Section planes — slicing a model to peer inside.

A section plane is an object in the model: an origin + a unit normal. The
ACTIVE plane cuts the display — everything on the normal's side disappears
(``Reverse`` flips it). Only ONE plane can be the active cut in the model
context, the usual convention; placing a new plane makes it the active one.
Planes carry a name and a symbol shown on their frame.

The cut is display-only scaffolding, never topology: geometry is untouched,
the renderer clips (``gl_ClipDistance``), picks/snaps filter by the plane,
and the sheet composer's hidden-line pass clips before projecting — that is
the whole point of the track (real plans and cross-sections on the sheets).
"""
from __future__ import annotations

from core.doc_model import Record, record_fields

import ingetrazo_core as _NATIVE
from ingetrazo_core import Vec3 as QVector3D

_last_uid = 0

#: How a cut can be filled: solid, hatched at 45°, cross-hatched, not at
#: all, or each layer with its own hatch (``"layers"``, as the sheets draw
#: it).
FILL_PATTERNS = ("solid", "hatch", "cross", "none", "layers")
#: Hatch spacing when neither the plane nor anything else says: 5 cm in
#: the model, a wall of 20 cm reads as four lines.
DEFAULT_HATCH_SPACING = 0.05
#: The layers' hatches in the model are laid as a sheet at 1:50 lays them —
#: the usual scale of a plan or a section with its materials; bricks,
#: blocks and their joints keep their real size at any scale.
LAYER_HATCH_SCALE = 50.0
#: More hatch lines than this in one cut and the rest of its layers stay
#: solid: a pattern that fine reads as a tone anyway, and draws slowly.
LAYER_HATCH_MOST = 250000


#: ``effective_fill(plane, style)``: ``(pattern, (r, g, b), spacing)`` the cut
#: of ``plane`` is filled with, the plane's own settings over the style's; the
#: style's Section Fill switch is the master (off, nothing is filled).
effective_fill = _NATIVE.section_effective_fill


def _take_uid(wanted: int | None) -> int:
    """A fresh uid, or ``wanted`` (from a loaded file) while keeping the
    counter ahead so later fresh uids never collide."""
    global _last_uid
    if wanted is None:
        _last_uid += 1
        return _last_uid
    wanted = int(wanted)
    if wanted > _last_uid:
        _last_uid = wanted
    return wanted


#: ``next_symbol(planes)``: the letter the next section plane takes, A, B, C…
#: then AA, AB. Sections are named by letter and the mark reads «A … A» across
#: the plan (Rafael, 2026-09-16: «no suele ser un 1, sino A-A, B-B»).
next_symbol = _NATIVE.section_next_symbol


@record_fields('point', 'normal', 'name', 'symbol', 'active', 'uid', 'hidden', 'fill', 'layer')
class SectionPlane(Record):
    """Origin + unit normal (+ name/symbol). ``active`` = the cutting one;
    ``hidden`` = its frame is not drawn (nor picked) — the cut, if it is
    the active one, stays. Hidden and inactive is a plane switched off."""

    __slots__ = ("point", "normal", "name", "symbol", "active", "uid",
                 "hidden", "fill", "layer")

    def __init__(self, point: QVector3D, normal: QVector3D,
                 name: str = "", symbol: str = "",
                 active: bool = False, uid: int | None = None,
                 hidden: bool = False) -> None:
        self.point = QVector3D(point)
        n = QVector3D(normal)
        if n.length() < 1e-12:
            n = QVector3D(0.0, 0.0, 1.0)
        self.normal = n.normalized()
        self.name = name
        self.symbol = symbol
        self.active = bool(active)
        self.hidden = bool(hidden)
        #: This cut's own fill, over the display style's: ``{"pattern":
        #: "solid" | "hatch" | "cross" | "none", "color": (r, g, b),
        #: "spacing": metres}``, each key optional — what is missing comes
        #: from the style. ``None``: the style's fill as it is.
        self.fill = None
        #: Layer name (None = the default layer). A hidden layer takes the
        #: plane away and switches its cut off; a scene that hides it does
        #: the same (#316).
        self.layer = None
        #: Stable identity for scenes (.igz SavedView references) — survives
        #: reordering and deletion of OTHER planes.
        self.uid = _take_uid(uid)

    def flip(self) -> None:
        """Reverse: the cut hides the other side."""
        self.normal = -self.normal

    def side(self, p: QVector3D) -> float:
        """Signed distance: > 0 is the HIDDEN side (the normal's side)."""
        return QVector3D.dotProduct(self.normal, p - self.point)

    # ---- Serialisation (.igz) ------------------------------------------------
    def to_dict(self) -> dict:
        """The .igz entry, defaults left out (older readers ignore what they
        do not know: hidden, layer, fill)."""
        return _NATIVE.section_plane_archive(self)

    @classmethod
    def from_dict(cls, raw: dict) -> "SectionPlane":
        """A plane from its entry, its fill read defensively."""
        return _NATIVE.section_plane_load(cls, raw)

    def __repr__(self) -> str:  # pragma: no cover — debug aid
        state = " ACTIVE" if self.active else ""
        return f"SectionPlane({self.symbol or self.uid}{state})"


# ---- The cut, as geometry --------------------------------------------------
# In Rust (``rust/ingetrazo_core/src/model/section_fill.rs``):
#
# ``cut_chords(v0, e1, e2, normal, point, visible=None, rows=False,
# reach=None)``: the chords the plane cuts out of triangles given as ``v0`` +
# edge vectors ``e1``/``e2`` ((N, 3) arrays, as the pick index keeps them), an
# (K, 2, 3) float32 array or ``None``; ``rows=True`` also gives the triangle
# of each chord. ``PlaneReach(v0, e1, e2, normal)`` keeps how far along the
# normal each corner stands, so a plane slid along it (a section dragged by
# its arrow) only cuts the triangles it now crosses (``fits``, ``cut``).
#
# ``fill_triangles(chords, normal)``: the section fill as (T, 3, 3) float32
# triangles, ONLY the cut geometry: chords chained into closed rings
# (``core.hlr.section_loops``), an open chain dropped, a ring inside another
# a hole and one inside that hole solid again, rings that merely overlap
# both filled. ``hatch_segments(chords, normal, spacing, cross=False)``:
# (L, 2, 3) float32 lines at 45° (and 135°) clipped to those same areas.
PlaneReach = _NATIVE.PlaneReach
fill_triangles = _NATIVE.section_fill_triangles
hatch_segments = _NATIVE.section_hatch_segments


def layer_fills(chords, tags, normal, scene, color,
                scale_n: float = LAYER_HATCH_SCALE) -> list:
    """The cut filled layer by layer — each with its layer's hatch, as a
    sheet at 1:``scale_n`` draws it (bricks laid by the cut and the wall's
    thickness, timber, insulation…): a list of ``(triangles, rgb, lines,
    ink)`` — the (T, 3, 3) fill in ``rgb`` and the (L, 2, 3) hatch lines
    over it in ``ink``, world space. ``tags`` gives each chord's layer.

    A layer without a hatch, and one whose hatch sets no ink («as the
    frame says»), takes ``color`` — the plane's own fill; a hatch lies on
    its layer's ground, or on a pale tone of its ink (the paper of a
    sheet)."""
    import numpy as np
    from core.hatches import hatch_lines
    from core.hlr import section_loops
    from core.layers import cut_hatch_forms, cut_hatches, hatch_ground_rgb,         hatch_ink_rgb
    if chords is None or not len(chords):
        return []
    # A solid of more than one layer is filled as the layer of most of it.
    tags = np.asarray(_NATIVE.hlr_closing_tags(chords, tags), dtype=object)
    hatches = cut_hatches(scene)
    forms = cut_hatch_forms(scene, scale_n)
    u, v = _NATIVE.section_sheet_basis(normal)
    n = np.cross(u, v)
    k = 1000.0 / float(scale_n or LAYER_HATCH_SCALE)    # paper mm per metre
    cut = "plan" if abs(normal.z()) >= 0.7 else "vertical"
    color = tuple(float(c) for c in color[:3])
    plain: list = []
    out: list = []
    budget = LAYER_HATCH_MOST
    for tag in sorted({str(t) for t in tags}):
        mine = chords[tags == tag]
        key = hatches.get(tag)
        if not key:
            plain.append(mine)
            continue
        groups, _basis = _NATIVE.section_fill_groups(mine, normal)
        tris = _NATIVE.section_group_triangles(groups)
        if not len(tris):
            continue
        ly = scene.layer(tag) if hasattr(scene, "layer") else None
        ink = tuple((hatch_ink_rgb(ly) if ly is not None else None)
                    or color)
        if key == "solid":
            out.append((tris, ink, None, ink))
            continue
        ground = hatch_ground_rgb(ly) if ly is not None else None
        ground = tuple(ground) if ground is not None else tuple(
            c + (1.0 - c) * 0.85 for c in ink)
        form = dict(forms.get(tag, {}), cut=cut)
        off = float(np.dot(groups[0][0][0][0], n)) * n
        segs: list = []
        for rings3, _rings2 in groups:
            if budget <= 0:
                break
            paper = [[(float(p @ u) * k, -float(p @ v) * k) for p in r]
                     for r in rings3]
            got = hatch_lines(key, paper, form)
            budget -= len(got)
            segs += got
        lines = None
        if segs:
            a = np.asarray(segs, dtype=np.float64) / k
            lines = np.stack([off + a[:, 0:1] * u - a[:, 1:2] * v,
                              off + a[:, 2:3] * u - a[:, 3:4] * v], axis=1)
            lines = lines.astype(np.float32)
        out.append((tris, ground, lines, ink))
    if plain:
        tris = _NATIVE.section_group_triangles(_NATIVE.section_fill_groups(np.concatenate(plain),
                                             normal)[0])
        if len(tris):
            out.insert(0, (tris, color, None, color))
    return out
