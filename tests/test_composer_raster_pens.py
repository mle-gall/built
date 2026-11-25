# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Raster frames take the Edges / Profiles pens too: a GL line is one pixel
whatever the dpi, and one pixel at 300 dpi is a 0.085 mm hairline that
vanishes on paper and on the composer's scaled preview (Marco, 2026-09-14:
«las líneas de la topografía casi no se ven»). The render thickens its
lines to the pen by redrawing them at sub-pixel offsets."""
from types import SimpleNamespace

from PySide6.QtWidgets import QWidget

from core.composition import RENDER_DPI, pen_px
from tests.test_composer_canvas import _FakeViewport
from tests.controller_surface import ControllerSurface as Viewport


def test_pen_px_rounds_a_paper_width_to_render_pixels():
    assert pen_px(0.18, 300) == 2
    assert pen_px(0.35, 300) == 4
    assert pen_px(0.05, 300) == 1          # never thinner than the hairline
    assert pen_px(0.0, 300) == 1


def test_line_jitter_is_a_disc_of_subpixel_offsets_for_exports_only():
    live = SimpleNamespace(_export_size=None)
    assert Viewport._line_jitter(live, 3, 300, 200) == [(0.0, 0.0)]
    export = SimpleNamespace(_export_size=(300, 200))
    assert Viewport._line_jitter(export, 1, 300, 200) == [(0.0, 0.0)]
    two = Viewport._line_jitter(export, 2, 300, 200)
    assert len(two) == 4                  # the four half-pixel corners
    assert {(abs(round(dx * 300 / 2, 2)), abs(round(dy * 200 / 2, 2)))
            for dx, dy in two} == {(0.5, 0.5)}
    three = Viewport._line_jitter(export, 3, 300, 200)
    assert len(three) == 5                # a cross: centre + four neighbours
    assert (0.0, 0.0) in three


class _RecordingViewport(_FakeViewport):
    def __init__(self):
        super().__init__()
        self.plano_style = None
        self.style_override = None
        self._export_edge_px = 1
        self._export_profile_px = 1
        self.seen = []

    def _effective_style(self):
        from core.style import Style
        return self.style_override or Style()

    def render_image(self, w, h, overlays=True, sharp_shadows=False):
        self.seen.append((self._export_edge_px, self._export_profile_px))
        return None


