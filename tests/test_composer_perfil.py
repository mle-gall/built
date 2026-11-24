# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The terrain-profile sheet item (Marco, 2026-09-03: "perfil de terreno en
láminas"): elevation under a traced path against chainage, with a horizontal
scale, a vertical exaggeration and a grid — sampled from the survey or the
DEM at paint time, saved with the sheet as what to plot and how."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import QRectF
from PySide6.QtGui import QImage, QPainter
from ingetrazo_core import Vec3 as QVector3D
from PySide6.QtWidgets import QApplication, QWidget

from core.composition import AddItemCommand, Composicion, PerfilTerreno
from georef.datum import SceneDatum
from georef.geopath import GeoPath
from tests.test_composer_canvas import _FakeViewport

_app = QApplication.instance() or QApplication([])


def V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


class StubSampler:
    """Elevation = f(x, y): a 10 % grade along x, no tiles, no network."""
    def __init__(self, datum):
        self.datum = datum

    def ensure_area(self, *_a):
        pass

    def elevation_at_local(self, p):
        return 2500.0 + 0.10 * p.x() + 0.02 * p.y()


def _paint(item):
    img = QImage(800, 400, QImage.Format_ARGB32)
    img.fill(0xFFFFFFFF)
    painter = QPainter(img)
    painter.scale(4.0, 4.0)
    item.paint(painter, None, None)
    painter.end()
    return img


def test_the_profile_travels_with_the_sheet():
    comp = Composicion()
    comp.perfiles.append(PerfilTerreno(x_mm=30, y_mm=40, w_mm=150, h_mm=60,
                                       path_index=2, scale_n=1000.0, exag=10.0,
                                       grid_h_m=20.0, title="Perfil eje"))
    d = comp.to_dict()
    assert d["perfiles"][0]["scale_n"] == 1000.0
    back = Composicion.from_dict(d)
    assert len(back.perfiles) == 1
    assert back.perfiles[0].path_index == 2 and back.perfiles[0].exag == 10.0
    assert back.perfiles[0].title == "Perfil eje"
    assert back.perfiles[0] in back.all_items()


