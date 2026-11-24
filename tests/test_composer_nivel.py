# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Level marks (sheets plan 2026-09-05, point 8): «N.P.T. +0.15» beside a
triangle (sections) or a quartered circle (plans), anchored to a model
point whose height they read and follow."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
from PySide6.QtGui import QColor, QImage, QPainter
from ingetrazo_core import Vec3 as QVector3D
from PySide6.QtWidgets import QApplication, QWidget

from core.composition import Composicion, NivelItem
from tests.test_composer_canvas import _FakeViewport

_app = QApplication.instance() or QApplication([])
V = QVector3D


def test_the_value_reads_signed_with_a_datum_and_plus_minus_zero():
    nv = NivelItem(z_m=0.15)
    assert nv.units == "m"
    assert nv.label() == "N.P.T. +0.15"
    nv.z_m = -0.3
    assert nv.value_text() == "-0.30"
    nv.z_m = 0.004                         # zero at two decimals
    assert nv.value_text() == "±0.00"
    nv.decimals = 3
    assert nv.value_text() == "+0.004"
    nv.text = "NIVEL"                      # no {z}: appended
    nv.z_m, nv.decimals = 2.5, 2
    assert nv.label() == "NIVEL +2.50"
    nv.datum_m = 2.5                       # the datum makes it the ±0.00
    assert nv.label() == "NIVEL ±0.00"
    nv.anchor_uid, nv.a_world = "f", [1.0, 2.0, 3.7]   # anchored: reads Z
    assert nv.anchored and nv.level_m() == pytest.approx(1.2)
    assert nv.label() == "NIVEL +1.20"


def test_a_level_mark_writes_its_own_unit_with_the_same_signs():
    """Issue #403: a mark stores the unit it was born with. Metres keep the
    bare «+0.15» every existing sheet shows; any other unit is the same
    sign in front of the shared length format."""
    feet = NivelItem(z_m=0.15, units="ft", decimals=2)
    assert feet.value_text() == "+0.49'"
    inches = NivelItem(z_m=0.0381, units="in", decimals=2)
    assert inches.value_text() == '+1.50"'
    mixed = NivelItem(z_m=0.15, units="ft-in", decimals=2)
    assert mixed.value_text() == '+0\'5.91"'
    below = NivelItem(z_m=-0.3, units="ft-in", decimals=2)
    assert below.value_text() == '-0\'11.81"'
    zero = NivelItem(z_m=0.0, units="ft-in", decimals=2)
    assert zero.value_text() == '±0\'0.00"'
    cm = NivelItem(z_m=0.15, units="cm", decimals=2)
    assert cm.value_text() == "+15.00 cm"
    assert NivelItem(z_m=0.0, units="cm").value_text() == "±0.00 cm"
    # a sheet saved before the field existed is still a metre mark
    old = Composicion.from_dict({"niveles": [{"z_m": 0.15}]}).niveles[0]
    assert old.units == "m" and old.label() == "N.P.T. +0.15"
    gone = Composicion.from_dict(Composicion(
        niveles=[NivelItem(z_m=0.3048, units="ft", decimals=1)]).to_dict())
    assert gone.niveles[0].units == "ft"
    assert gone.niveles[0].value_text() == "+1.0'"


def _dark(img, x0, y0, x1, y1):
    return sum(1 for x in range(max(0, int(x0)), min(img.width(), int(x1)))
               for y in range(max(0, int(y0)), min(img.height(), int(y1)))
               if img.pixelColor(x, y).lightness() < 140)


