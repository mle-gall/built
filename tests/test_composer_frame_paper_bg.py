# SPDX-License-Identifier: GPL-3.0-or-later
"""The frame's «Paper background»: render on white with no sky whatever
the style says (Marco, 2026-09-08, the rebar in X-ray: «no me gusta que
tenga el fondo gris del model»)."""
from PySide6.QtWidgets import QWidget

from core.composition import Composicion, MarcoVista
from tests.test_composer_canvas import _FakeViewport


class _RecordingViewport(_FakeViewport):
    """Records the style the render ran with, like the GL viewport would."""

    def __init__(self):
        super().__init__()
        self.plano_style = None
        self.style_override = None
        self.seen = []

    def _effective_style(self):
        from core.style import Style
        return self.style_override or Style()

    def render_image(self, w, h, overlays=True, sharp_shadows=False):
        self.seen.append(self._effective_style())
        return None


def test_paper_bg_round_trips_and_defaults_off():
    comp = Composicion()
    comp.frames = [MarcoVista(paper_bg=True)]
    again = Composicion.from_dict(comp.to_dict())
    assert again.frames[0].paper_bg is True
    assert MarcoVista(**{"x_mm": 1.0}).paper_bg is False       # old .igz


