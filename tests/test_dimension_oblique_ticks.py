# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Oblique ticks at 45° to the dimension line, «/» on a horizontal one, as
the technical-drawing standards draw them (#741). The sheet drew «\\»,
135° to the line, and the 3D view drew them square to it."""
from __future__ import annotations

from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
import math

import pytest
from PySide6.QtGui import QImage, QPainter
from ingetrazo_core import Vec3 as QVector3D


from core.composition import CotaAngularItem, CotaItem, CotaRadialItem  # noqa: E402


class _Recorder(QPainter):
    """A real painter that also keeps every line it is asked to draw."""

    def __init__(self, device):
        super().__init__(device)
        self.lines = []

    def drawLine(self, *args):
        if len(args) == 2:
            a, b = args
            self.lines.append((a.x(), a.y(), b.x(), b.y()))
        else:
            self.lines.append(tuple(float(v) for v in args[:4]))
        super().drawLine(*args)


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


def _paint(fn, item):
    img = QImage(400, 400, QImage.Format_ARGB32)
    img.fill(0xFFFFFFFF)
    p = _Recorder(img)
    p.translate(200, 200)
    fn(p, item)
    p.end()
    return p.lines


def test_3d_view_draws_oblique_ticks_not_square_ones(native_window, monkeypatch):
    from core.dimension import Dimension
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
    img = QImage(400, 400, QImage.Format_ARGB32)
    img.fill(0xFFFFFFFF)
    p = _Recorder(img)
    vp._draw_dimensions(p)
    p.end()
    ticks = _ticks(p.lines, 8.0)
    assert len(ticks) == 2 and all(_is_slash(t) for t in ticks)
