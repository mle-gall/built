# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Two-point placement on the composer canvas: the sheet tools accept BOTH a
drag and click-move-click (the model's dimension-tool habit), snapping every
point. The dimension tool adds a THIRD click that pulls the dimension line
away from the measured points (``sep_mm``, as sheets usually do). A bare
click with a two-point tool must NOT place a zero-size item — that was 'the second point
never snaps': the first release placed a zero cota and silently disarmed the
tool."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
from PySide6.QtCore import QEvent, QPoint, QPointF, Qt
from PySide6.QtGui import QMouseEvent
from PySide6.QtWidgets import QApplication, QGraphicsScene

_inst = QApplication.instance()
if _inst is None:
    _app = QApplication([])
elif not isinstance(_inst, QApplication):
    pytest.skip("a non-widget QGuiApplication is already active",
                allow_module_level=True)

from core.composition import (Cajetin, Composicion, CotaItem,   # noqa: E402
                              FormaItem, MarcoVista)


def _mouse(view, etype, px, py, button=Qt.LeftButton,
           buttons=Qt.NoButton, mods=Qt.NoModifier):
    pos = QPointF(px, py)
    # The global point is where that viewport pixel is on the screen: the
    # scene finds what is under a press through it, so passing the local
    # point again only worked while the page sat at the window's left edge.
    glob = QPointF(view.viewport().mapToGlobal(pos.toPoint()))
    ev = QMouseEvent(etype, pos, glob, button, buttons, mods)
    if etype == QEvent.MouseButtonPress:
        view.mousePressEvent(ev)
    elif etype == QEvent.MouseMove:
        view.mouseMoveEvent(ev)
    else:
        view.mouseReleaseEvent(ev)


def _click(view, px, py):
    _mouse(view, QEvent.MouseButtonPress, px, py)
    _mouse(view, QEvent.MouseButtonRelease, px, py)


def _scene_xy(view, px, py):
    p = view.mapToScene(QPoint(px, py))
    return p.x(), p.y()


class _FakeViewport:
    """A GL-free stand-in: the composer only needs scene, camera, update."""

    def __init__(self):
        from core.camera import OrbitCamera
        from core.scene import Scene
        self.scene = Scene()
        self.camera = OrbitCamera()

    def update(self):
        pass


class TestCotaModel:
    def test_normal_is_perpendicular_and_unit(self):
        ct = CotaItem(dx_mm=30.0, dy_mm=40.0)
        nx, ny = ct.normal()
        assert nx * 30.0 + ny * 40.0 == pytest.approx(0.0)
        assert nx * nx + ny * ny == pytest.approx(1.0)

    def test_label_honours_decimals(self):
        ct = CotaItem(dx_mm=34.56, dy_mm=0.0, scale_n=100.0, decimals=1)
        assert ct.label() == "3.5 m"
        ct.decimals = 3
        assert ct.label() == "3.456 m"

    def test_old_documents_load_without_the_new_fields(self):
        ct = CotaItem(**{"x_mm": 5.0, "y_mm": 6.0, "dx_mm": 40.0,
                         "dy_mm": 0.0, "scale_n": 50.0, "offset_mm": 4.0,
                         "text": ""})
        assert ct.sep_mm == 0.0                              # line on the points
        assert ct.ends == "tick"
        assert ct.text_mm == pytest.approx(2.8)


