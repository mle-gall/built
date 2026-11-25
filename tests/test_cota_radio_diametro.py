# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Cota de radio y de diámetro — Fase 9 (Rafael).

He asked for the radius right after we shipped Fillet — «ya que pusisteis
el redondeo, pues sería lo suyo» (revisión 2, 42:40) — and his video on
drafting standards adds the diameter and the rules both obey, drawn case
by case on the reference sheet of `rafael-cotas/fotogramas/f1230.jpg`:

  8  the diameter's line passes through the centre
  9  it all fits inside → inside, arrows out at the arc
  10 it does not → arrows and words outside, and the line is prolonged
  12 the radius always starts AT the centre, text above, with its symbol
  13 changing quadrant must not leave the text upside down
  14 a radius that does not fit: out, arrow back at the centre, text above
  15 forbidden: an arrow pointing outwards on a leader that stops short of
     the centre, and text + arrow with no extension to the centre
  16 an arc is dimensioned exactly like a circle
"""
from __future__ import annotations

import math
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest

from built_core.composition import CotaRadialItem, readable_deg


def _r(**kw) -> CotaRadialItem:
    base = dict(x_mm=100.0, y_mm=80.0, radius_mm=20.0, angle_deg=-30.0,
                scale_n=50.0, text_mm=2.5)
    base.update(kw)
    return CotaRadialItem(**base)


# ---- what the label says --------------------------------------------------

def test_the_symbol_goes_with_the_value():
    assert _r().label() == "R1.00 m"                    # 20 mm at 1:50
    assert _r(kind="diameter").label() == "Ø2.00 m"
    assert _r(text="<> (typ.)").label() == "R1.00 m (typ.)"


def test_a_diameter_measures_twice_the_radius():
    a = _r()
    b = _r(kind="diameter")
    assert b.measured_mm() == pytest.approx(2 * a.measured_mm())
    assert b.real_distance_m() == pytest.approx(2.0)


# ---- rules 8, 12, 15, 16: the line reaches the centre --------------------

def test_a_radius_starts_at_the_centre():
    (ax, ay), (bx, by) = _r().line_points()
    assert (ax, ay) == (0.0, 0.0)                       # the centre itself
    assert math.hypot(bx, by) == pytest.approx(20.0)    # out to the arc


def test_a_diameter_runs_right_through_the_centre():
    (ax, ay), (bx, by) = _r(kind="diameter").line_points()
    assert (ax + bx, ay + by) == pytest.approx((0.0, 0.0))   # symmetric
    assert math.hypot(bx - ax, by - ay) == pytest.approx(40.0)


@pytest.mark.parametrize("deg", [0.0, -30.0, 90.0, 140.0, -170.0, 45.0])
def test_the_line_leaves_the_centre_in_any_direction_asked(deg):
    """Rule 12: «siempre parte del centro, en la dirección que se quiera»."""
    (ax, ay), (bx, by) = _r(angle_deg=deg).line_points()
    assert (ax, ay) == (0.0, 0.0)
    assert math.degrees(math.atan2(by, bx)) == pytest.approx(deg, abs=1e-9)


# ---- rules 9, 10, 14: inside while it fits, outside when it does not -----

def test_a_big_circle_keeps_the_words_inside():
    ct = _r(radius_mm=40.0)
    assert not ct.outside()
    lx, ly = ct.text_anchor()
    assert math.hypot(lx, ly) < ct.radius_mm            # over the line
    assert ct.tail_end() == pytest.approx(ct.line_points()[1])


def test_a_small_one_takes_them_out_and_prolongs_the_line():
    ct = _r(radius_mm=3.0)
    assert ct.outside()
    lx, ly = ct.text_anchor()
    assert math.hypot(lx, ly) > ct.radius_mm            # past the arc
    tail = math.hypot(*ct.tail_end())
    assert tail > ct.radius_mm                          # «la línea se prolonga»
    assert tail >= math.hypot(lx, ly)                   # and carries the words


@pytest.mark.parametrize("deg", [-30.0, 150.0, 0.0, 180.0, 60.0, -120.0])
def test_a_diameters_words_keep_off_the_centre_mark_on_the_right(deg):
    """Rule 3's reason: the centre belongs to the axes, and «un número
    encima de un eje se lee mal, sobre todo la coma». The Ø41,96 of the
    reference sheet sits in the outer half — and Marco fixed WHICH half
    (2026-09-20): the right-hand one, «no al medio porque se cruzaría con
    otras líneas de dibujo que salen del radio», whichever way the line
    was drawn."""
    ct = _r(kind="diameter", radius_mm=40.0, angle_deg=deg)
    assert not ct.outside()
    lx, ly = ct.text_anchor()
    assert math.hypot(lx, ly) == pytest.approx(20.0)     # half the radius
    assert lx > 0                                        # the right-hand half


def test_a_vertical_diameters_words_sit_on_its_top_half():
    """A vertical diameter's text reads bottom-to-top, so its «right» is
    the top of the page (y grows downward)."""
    for deg in (90.0, -90.0):
        lx, ly = _r(kind="diameter", radius_mm=40.0,
                    angle_deg=deg).text_anchor()
        assert abs(lx) < 1e-9 and ly < 0


def test_the_drafter_can_force_it_either_way():
    assert _r(radius_mm=3.0, placement="in").outside() is False
    assert _r(radius_mm=40.0, placement="out").outside() is True


# ---- rule 13: the text never ends up upside down -------------------------

@pytest.mark.parametrize("deg", [0.0, 30.0, 89.0, 91.0, 150.0, -150.0,
                                 -91.0, -89.0, 179.0])
def test_whatever_quadrant_it_is_in_the_text_stays_readable(deg):
    ct = _r(angle_deg=deg)
    ux, uy = ct.heading()
    turn = readable_deg(ux, uy)
    assert -90.0 <= turn < 90.0


# ---- it survives the file ------------------------------------------------

def test_it_travels_in_the_sheet_and_an_older_one_simply_has_none():
    from built_core.composition import Composicion
    c = Composicion()
    c.cotas_rad.append(_r(kind="diameter", radius_mm=7.0))
    back = Composicion.from_dict(c.to_dict())
    assert len(back.cotas_rad) == 1
    assert back.cotas_rad[0].kind == "diameter"
    assert back.cotas_rad[0].label() == "Ø0.70 m"
    assert Composicion.from_dict({}).cotas_rad == []
    # and it is one of the sheet's items, so z-order, groups, lock and
    # delete reach it like everything else
    assert back.cotas_rad[0] in back.all_items()


# ---- and the tool places it ----------------------------------------------


# ---- the angular dimension finally snaps, and Shift gives an exact 90° ---


# ---- the level mark reads its own height --------------------------------


# ---- one click ON the arc: the circle lends its centre and radius --------


