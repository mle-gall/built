# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Sheet cotas: the label below its line, turned to read the other way,
and the text–line gap set from the panel."""
from __future__ import annotations

from dataclasses import asdict, replace

from core.composition import Composicion, CotaItem

PX = 20


def _inked(img, x, y) -> bool:
    return (img.pixel(x, y) & 0xFF) < 128


def test_flip_travels_in_the_file():
    comp = Composicion()
    comp.cotas.append(CotaItem(text_flip=True, text_pos="below",
                               offset_mm=1.7))
    again = Composicion.from_dict(comp.to_dict()).cotas[0]
    assert (again.text_flip, again.text_pos, again.offset_mm) == (
        True, "below", 1.7)


