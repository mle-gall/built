# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Acotación desde línea base, y la regla 4 de la normativa.

Marco, 2026-09-19, after we closed the 0.4.7: «tal vez alguna como
dimensión alineada, otra [lineal] y otra dimcontinue». He was right that
the family is the way to read what Rafael wanted, and reading the video's
transcript settles which of them were missing: the standards video never
names a command, so DIMALIGNED / DIMLINEAR / DIMCONTINUE were already
there — and **DIMBASELINE was not**. It is the one that puts every
dimension from the SAME first point, each stacked a row further out,
which is how a plan carries accumulated distances off a corner.

Rule 4 of the standards video (08:00): «la línea de cota se prolonga
hasta cubrir todo el texto; si el texto crece, la línea crece». With the
text outside an end the line used to stop at the measured point and leave
the number floating beside it, which rule 6 calls «prohibido».
"""
from __future__ import annotations

import math


import pytest
from built_core import ScreenPoint as QPointF

from core.composition import CotaItem


# ---- rule 4 -------------------------------------------------------------

def _ct(**kw) -> CotaItem:
    base = dict(x_mm=20.0, y_mm=40.0, dx_mm=30.0, dy_mm=0.0, sep_mm=0.0,
                scale_n=50.0, text_mm=3.0)
    base.update(kw)
    return CotaItem(**base)


def test_the_words_over_the_line_need_no_prolongation():
    assert _ct(text_along="middle").text_tail() is None


@pytest.mark.parametrize("along,side", [("end", 1.0), ("start", -1.0)])
def test_the_line_runs_on_under_words_placed_outside(along, side):
    ct = _ct(text_along=along)
    tail = ct.text_tail()
    assert tail is not None
    (ax, ay), (bx, by) = tail
    (l0x, l0y), (l1x, l1y) = ct.line_points()
    start = (l1x, l1y) if side > 0 else (l0x, l0y)
    assert (ax, ay) == pytest.approx(start)          # it leaves the line's end
    assert math.copysign(1.0, bx - ax) == side       # …going outwards
    # and it is long enough to run under the whole label
    assert abs(bx - ax) >= ct.label_extent_mm()


def test_if_the_text_grows_the_line_grows():
    """«Si el texto crece, la línea crece» — the reason it is measured
    from the label and not from a constant."""
    short = _ct(text_along="end", text="1")
    long = _ct(text_along="end", text="PROFUNDIDAD VARIABLE")
    a = short.text_tail()
    b = long.text_tail()
    assert abs(b[1][0] - b[0][0]) > abs(a[1][0] - a[0][0]) + 10.0


def test_a_hand_dragged_label_leaves_the_line_where_it_was():
    """Sheet layout programs let the box be dragged anywhere and the line stays put;
    rule 4 is about the words sitting outside an END, not about a drag."""
    assert _ct(text_along="end", text_dx_mm=8.0).text_tail() is None


def test_a_vertical_cota_prolongs_along_ITS_line():
    ct = _ct(dx_mm=0.0, dy_mm=-30.0, text_along="end")
    (ax, ay), (bx, by) = ct.text_tail()
    assert ax == pytest.approx(bx)                   # straight up the page
    assert by < ay


# ---- DIMBASELINE ---------------------------------------------------------

def _run(comp, tool, pts, offset):
    """Drive a chain / baseline run: two points, the offset, then the rest."""
    view = comp._view
    comp.tool_mode = tool
    view._chain_click(QPointF(*pts[0]), None)
    view._chain_click(QPointF(*pts[1]), None)
    view._chain_click(QPointF(*offset), None)
    for p in pts[2:]:
        view._chain_click(QPointF(*p), None)
    return view


# ---- AutoCAD's flow: a run carries on from a dimension already there ----

def _armed(comp, tool):
    comp.tool_mode = tool
    return comp._view


