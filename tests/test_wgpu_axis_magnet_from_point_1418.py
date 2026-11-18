# SPDX-License-Identifier: GPL-3.0-or-later
"""Upstream #1418 on the Metal view: drawing along green with the axis
magnet, 'From point' still lines the end up with the origin, so the line
ends exactly on the red axis without the arrow lock; away from every
reference the magnet still slides.

Drives an unshown ``WgpuView`` with the real Line tool (the fixture of
``test_.skp_inference_gaps``)."""
import pytest
from PySide6.QtCore import QPointF, Qt
from PySide6.QtGui import QMouseEvent, QVector3D as V
from PySide6.QtWidgets import QApplication

from tests.test_.skp_inference_gaps import click, refresh, view  # noqa: F401


def _open_rectangle(view):
    view.scene.mesh.add_edge(V(0, 0, 0), V(0, 6, 0))
    view.scene.mesh.add_edge(V(0, 6, 0), V(10, 6, 0))
    view.camera.target = V(5, 3, 0)
    view.camera.distance = 22
    refresh(view)


def _move_px(view, x, y):
    q = QPointF(x, y)
    QApplication.sendEvent(view, QMouseEvent(
        QMouseEvent.MouseMove, q, q, Qt.NoButton, Qt.NoButton, Qt.NoModifier))
    return view.last_snap


@pytest.mark.parametrize("off_px", [6.0, 3.0, 0.0])
def test_metal_right_side_ends_exactly_on_the_red_axis(view, off_px):
    _open_rectangle(view)
    click(view, V(10, 6, 0))
    assert (view.tools.start_point - V(10, 6, 0)).length() < 1e-6
    p = view._world_to_pixel(V(10, 0, 0))
    snap = _move_px(view, p[0] + off_px * 0.7, p[1] + off_px * 0.7)
    # The Metal view hands in the edges near the cursor only, so the line-up
    # comes from the origin reference itself ("origin"), not from the end of
    # the far green edge that sits on it ("from_point" on GL).
    assert snap.kind in ("from_point", "origin"), (snap.kind, snap.point)
    assert snap.point.x() == pytest.approx(10.0, abs=1e-6)
    assert snap.point.y() == pytest.approx(0.0, abs=1e-6)      # ON red
    assert snap.guide is not None and (snap.guide[0] - V(0, 0, 0)).length() < 1e-9


def test_metal_magnet_still_slides_away_from_references(view):
    _open_rectangle(view)
    click(view, V(10, 6, 0))
    p = view._world_to_pixel(V(10, 1.7, 0))
    snap = _move_px(view, p[0] + 2, p[1])
    assert snap.kind == "axis", (snap.kind, snap.point)
    assert snap.point.x() == pytest.approx(10.0, abs=1e-6)
    assert snap.point.y() == pytest.approx(1.7, abs=0.15)
