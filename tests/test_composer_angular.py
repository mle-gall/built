# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The composer's angular dimension (the usual Angular Dimension tool)."""
from __future__ import annotations

import math

import pytest
from PySide6.QtCore import QEvent, QPointF, Qt
from PySide6.QtGui import QImage, QKeyEvent, QMouseEvent, QPainter

from core.composition import (AddItemCommand, Composicion, CotaAngularItem,
                              RemoveItemCommand)


def test_angle_is_the_short_way_round_and_labels_follow_decimals():
    ca = CotaAngularItem(ax_mm=30, ay_mm=0, bx_mm=0, by_mm=-30)   # right, up
    a0, sweep = ca.angles()
    assert a0 == pytest.approx(0.0)
    assert sweep == pytest.approx(-math.pi / 2)        # counter-clockwise on screen
    assert ca.angle_deg() == pytest.approx(90.0)
    assert ca.label() == "90.0°"
    ca.decimals = 0
    assert ca.label() == "90°"
    ca.text = "α = <>"
    assert ca.label() == "α = 90°"
    # 350° apart the short way is 10°, whichever ray comes first.
    wide = CotaAngularItem(ax_mm=30, ay_mm=0,
                           bx_mm=30 * math.cos(math.radians(350)),
                           by_mm=30 * math.sin(math.radians(350)))
    assert wide.angle_deg() == pytest.approx(10.0)


def test_angular_cota_lives_in_the_composition_and_round_trips():
    c = Composicion()
    ca = CotaAngularItem(x_mm=50, y_mm=60, radius_mm=12.5, text="<> aprox.")
    AddItemCommand(c, ca).do()
    assert c.cotas_ang == [ca] and ca in c.all_items()
    back = Composicion.from_dict(c.to_dict())
    assert len(back.cotas_ang) == 1
    assert (back.cotas_ang[0].radius_mm, back.cotas_ang[0].text) == (12.5, "<> aprox.")
    RemoveItemCommand(c, ca).do()
    assert c.cotas_ang == []


def _press(view, x, y, button=Qt.LeftButton):
    pos = QPointF(x, y)
    view.mousePressEvent(QMouseEvent(QEvent.MouseButtonPress, pos, pos,
                                     button, button, Qt.NoModifier))
    view.mouseReleaseEvent(QMouseEvent(QEvent.MouseButtonRelease, pos, pos,
                                       button, Qt.NoButton, Qt.NoModifier))


