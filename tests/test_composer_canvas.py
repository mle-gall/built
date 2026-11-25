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

import pytest

from built_core.composition import (Cajetin, Composicion, CotaItem,   # noqa: E402
                              FormaItem, MarcoVista)


class _FakeViewport:
    """A GL-free stand-in: the composer only needs scene, camera, update."""

    def __init__(self):
        from built_core.camera import OrbitCamera
        from built_core.scene import Scene
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


