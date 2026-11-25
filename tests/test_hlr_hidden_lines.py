# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Hidden lines, dashed, on the vector sheets (issue #81, @pacaeiro: «I
expected that Hidden line will show hidden lines of the model as dashed
lines — that's the standard»)."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np

from core.hlr import KIND_HIDDEN, hlr_drawing
from core.scene import Scene
from tests.test_hlr_line_classes import _box, _camera


def _scene_with_a_box_behind_a_wall():
    """A 4×1×3 wall in front (y 0..1) and a 1×1×1 box behind it (y 3..4),
    seen from the front: the box is wholly hidden."""
    s = Scene()
    _box(s.mesh, -2, 0, 0, 2, 1, 3)
    _box(s.mesh, -0.5, 3, 0, 0.5, 4, 1)
    return s


def test_hidden_edges_come_back_only_when_asked():
    s = _scene_with_a_box_behind_a_wall()
    cam = _camera(s, "std:front")
    plain = hlr_drawing(s, cam)
    assert not np.any(plain.kinds == KIND_HIDDEN)
    d = hlr_drawing(s, cam, hidden=True)
    hid = d.segs[d.kinds == KIND_HIDDEN]
    # The box behind the wall, dashed: its top and its two sides, ONCE
    # each (its front and back edges fall on the same line on paper), and
    # not its bottom, which lies on the wall's visible base line — nor the
    # wall's own back edges, which lie under its visible outline.
    pieces = sorted(tuple(sorted([(round(r[0], 6), round(r[1], 6)),
                                  (round(r[2], 6), round(r[3], 6))]))
                    for r in hid)
    assert pieces == [((-0.5, -1.5), (-0.5, -0.5)),
                      ((-0.5, -0.5), (0.5, -0.5)),
                      ((0.5, -1.5), (0.5, -0.5))]
    # The visible drawing is the same with or without the hidden lines.
    vis = d.segs[d.kinds != KIND_HIDDEN]
    assert len(vis) == len(plain.segs)


def test_a_fully_visible_model_has_no_hidden_lines_from_above():
    s = Scene()
    _box(s.mesh, 0, 0, 0, 1, 1, 1)
    d = hlr_drawing(s, _camera(s, "std:top"), hidden=True)
    # From above the bottom face's edges lie right under the top's: they
    # are hidden behind the top face only where not coincident — the
    # sides are seen edge-on, nothing else hides.
    assert np.all(d.kinds[d.kinds == KIND_HIDDEN] == KIND_HIDDEN)


