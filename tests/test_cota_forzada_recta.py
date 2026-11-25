# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Cota forzada recta — Fase 8 de la revisión de Rafael (41:30–43:00).

    «Si el punto a acotar no está perfectamente alineado con el otro… yo
    quiero acotar de aquí a aquí arriba, pero no tengo un punto
    perfectamente alineado con este que coincida. ¿Veis cómo queda la cota
    inclinada? La cota no puede quedar inclinada si estás acotando algo en
    vertical.»

Shift used to drag the second POINT onto an axis, which is the wrong cure:
it throws away the snap to the point he actually wants — «el problema es
que aquí me cogen muchos puntos, no me coge el punto final así como así» —
and leaves the number to the eye, «software técnico: a ojo no». What a
drafter wants is AutoCAD's DIMLINEAR: keep both points, measure only their
horizontal or vertical separation, and draw the line straight with
extension lines of different lengths.
"""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest

from built_core.composition import CotaItem, cota_line_deg


# Two points that are NOT aligned: 30 mm across, 40 mm up the page.
def _slanted(**kw) -> CotaItem:
    return CotaItem(x_mm=20.0, y_mm=80.0, dx_mm=30.0, dy_mm=-40.0,
                    sep_mm=12.0, scale_n=50.0, **kw)


def test_aligned_is_still_the_slanted_cota_of_before():
    ct = _slanted()
    (a2x, a2y), (b2x, b2y) = ct.line_points()
    assert (b2x - a2x, b2y - a2y) == pytest.approx((30.0, -40.0))
    assert ct.measured_mm() == pytest.approx(50.0)       # the hypotenuse
    assert ct.label() == "2.50 m"


def test_forced_vertical_measures_the_height_and_draws_straight():
    ct = _slanted(axis="v")
    (a2x, a2y), (b2x, b2y) = ct.line_points()
    assert a2x == pytest.approx(b2x)                     # a vertical line
    assert (b2y - a2y) == pytest.approx(-40.0)
    assert ct.measured_mm() == pytest.approx(40.0)
    assert ct.label() == "2.00 m"                        # 40 mm at 1:50
    assert cota_line_deg(ct) == pytest.approx(-90.0)     # and it reads ISO


def test_forced_horizontal_measures_the_width():
    ct = _slanted(axis="h")
    (a2x, a2y), (b2x, b2y) = ct.line_points()
    assert a2y == pytest.approx(b2y)
    assert ct.measured_mm() == pytest.approx(30.0)
    assert ct.label() == "1.50 m"
    assert cota_line_deg(ct) == pytest.approx(0.0)


def test_the_two_extension_lines_have_DIFFERENT_lengths():
    """That is the whole point: each one reaches its own real point, so a
    cota over two points at different heights is straight anyway."""
    ct = _slanted(axis="v")
    (a2x, a2y), (b2x, b2y) = ct.line_points()
    from math import hypot
    first = hypot(a2x - 0.0, a2y - 0.0)
    second = hypot(b2x - ct.dx_mm, b2y - ct.dy_mm)
    assert first == pytest.approx(12.0)                  # the near point
    assert second == pytest.approx(18.0)                 # the far one
    assert abs(first - second) > 1.0


def test_an_older_sheet_opens_aligned_and_a_forced_one_survives_the_file():
    from built_core.composition import Composicion
    c = Composicion()
    c.cotas.append(_slanted(axis="v"))
    c.cotas.append(_slanted())
    back = Composicion.from_dict(c.to_dict())
    assert [x.axis for x in back.cotas] == ["v", ""]
    # a cota written before the field simply has none
    legacy = Composicion.from_dict(
        {"cotas": [{"x_mm": 1.0, "y_mm": 2.0, "dx_mm": 30.0, "dy_mm": 40.0}]})
    assert legacy.cotas[0].axis == ""
    assert legacy.cotas[0].measured_mm() == pytest.approx(50.0)


