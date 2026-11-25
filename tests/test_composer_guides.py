# SPDX-License-Identifier: GPL-3.0-or-later
"""QGIS's rulers and guides in the composer (Marco, 2026-09-08: «en QGIS
muestran como unas guías… sería bueno implementar eso en composición»)."""
from core.input_types import QPointF
from PySide6.QtWidgets import QWidget

from core.composition import Composicion, FormaItem
from tests.test_composer_canvas import _FakeViewport


def test_guides_round_trip_in_the_sheet():
    comp = Composicion()
    comp.guides_v = [100.0, 40.5]
    comp.guides_h = [30.0]
    again = Composicion.from_dict(comp.to_dict())
    assert (again.guides_v, again.guides_h) == ([100.0, 40.5], [30.0])
    assert Composicion.from_dict({"name": "x"}).guides_v == []   # old .igz


