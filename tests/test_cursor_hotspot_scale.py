# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The winit cursor hotspot falls on the pencil tip at both raster scales."""
import numpy as np
import pytest
from built_render import tool_cursor_art


@pytest.mark.parametrize("side", [32, 64])
def test_the_pencil_draws_at_its_tip(side):
    rgba, hotspot = tool_cursor_art("line", side)
    alpha = np.frombuffer(rgba, np.uint8).reshape(side, side, 4)[..., 3]
    y, x = np.where(alpha > 128)
    tip = (y - x).argmax()
    hx, hy = (round(v * side / 48) for v in hotspot)
    assert abs(hx - x[tip]) <= side / 10 and abs(hy - y[tip]) <= side / 10
