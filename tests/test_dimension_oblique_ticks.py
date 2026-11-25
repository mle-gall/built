# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Oblique ticks at 45° to the dimension line, «/» on a horizontal one, as
the technical-drawing standards draw them (#741). The sheet drew «\\»,
135° to the line, and the 3D view drew them square to it."""
from __future__ import annotations

from built_render import AnnotationCanvas
from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
import math

import pytest
from built_core import Vec3 as QVector3D


from built_core.composition import CotaAngularItem, CotaItem, CotaRadialItem  # noqa: E402


class _Recorder:
    """A real painter that also keeps every line it is asked to draw."""

    def __init__(self):
        self.canvas = AnnotationCanvas(400, 400, 400)
        self.lines = []

    def __getattr__(self, name):
        return getattr(self.canvas, name)

    def drawLine(self, *args):
        if len(args) == 2:
            a, b = args
            self.lines.append((a.x(), a.y(), b.x(), b.y()))
        else:
            self.lines.append(tuple(float(v) for v in args[:4]))
        self.canvas.drawLine(*args)


def _ticks(lines, length):
    """The lines of the given length: the ticks, not the dimension line."""
    return [ln for ln in lines
            if math.hypot(ln[2] - ln[0], ln[3] - ln[1]) == pytest.approx(
                length, abs=0.01)]


def _is_slash(line):
    """«/» on screen (y down): rising to the right, at 45°."""
    x0, y0, x1, y1 = line
    dx, dy = x1 - x0, y1 - y0
    return abs(abs(dx) - abs(dy)) < 1e-6 and dx * dy < 0


def test_3d_view_draws_oblique_ticks_not_square_ones(native_window, monkeypatch):
    from built_core.dimension import Dimension
    win = native_window
    vp = win.viewport
    scene = vp.scene
    dim = Dimension(QVector3D(0, 0, 0), QVector3D(10, 0, 0),
                    QVector3D(0, 0, 0))
    scene.dimensions.append(dim)
    scene.dimension_style["ends"] = "tick"

    def to_px(p):
        return (100.0, 200.0) if p.x() < 5 else (300.0, 200.0)

    monkeypatch.setattr(vp, "_world_to_pixel", to_px)
    monkeypatch.setattr(vp, "_occluded_on_screen", lambda w: False)
    monkeypatch.setattr(vp, "_draw_occluded_segment", lambda *a: None)
    p = _Recorder()
    vp._draw_dimensions(p)
    ticks = _ticks(p.lines, 8.0)
    assert len(ticks) == 2 and all(_is_slash(t) for t in ticks)
