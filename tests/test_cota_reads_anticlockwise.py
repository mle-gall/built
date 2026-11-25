# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Regla 2 de la normativa de acotación (Rafael, «Rafa 3D», 09:00–10:00).

Dimension text is read from the bottom and from the right of the sheet —
you tilt your head to the LEFT, never to the right. It is the rule he
repeats most, and it is circular: it holds at every inclination.

The rule was written out at five call sites in two contradictory
spellings. Three tested ``deg > 90 or deg < -90`` and so let the exact
+90° of a cota drawn DOWNWARD through; two tested ``deg <= -90`` and so
turned every vertical the wrong way round — and that second pair is what
projects the MODEL's dimensions onto a sheet, which is where Rafael saw it.
"""
from __future__ import annotations


import pytest
from built_core import Vec3 as QVector3D

from core.composition import CotaItem, readable_deg


# ---- the rule itself ------------------------------------------------------

@pytest.mark.parametrize("dx,dy,want", [
    (10.0, 0.0, 0.0),        # horizontal, left to right
    (-10.0, 0.0, 0.0),       # …drawn right to left: same text
    (0.0, 10.0, -90.0),      # vertical drawn DOWN the page ← the one that leaked
    (0.0, -10.0, -90.0),     # vertical drawn UP the page
    (10.0, 10.0, 45.0),      # down to the right
    (-10.0, -10.0, 45.0),
    (10.0, -10.0, -45.0),    # up to the right
    (-10.0, 10.0, -45.0),
    (10.0, 30.0, 71.565051),     # steep: already legible, left alone
    (-10.0, 30.0, -71.565051),   # steep the other way: wraps a half-turn
    (0.0, 0.0, 0.0),             # a cota of no length says nothing
])
def test_the_text_never_reads_head_to_the_right(dx, dy, want):
    deg = readable_deg(dx, dy)
    assert deg == pytest.approx(want, abs=1e-6)
    assert -90.0 <= deg < 90.0          # the legible half-turn, half-open


def test_a_line_and_its_reverse_give_the_very_same_text_angle():
    """Which way the drafter happened to drag is not information."""
    for dx, dy in ((40.0, 0.0), (0.0, 40.0), (30.0, 17.0), (-8.0, 25.0)):
        assert readable_deg(dx, dy) == pytest.approx(readable_deg(-dx, -dy))


# ---- reglas 1, 5 y 18: la norma del DOCUMENTO ----------------------------


def test_the_standard_lives_in_the_document_and_defaults_to_iso(tmp_path):
    """It travels in the .igz, and an older file — which has no such key —
    opens as ISO, which is what those drawings were made to."""
    from pathlib import Path
    from core.scene import Scene
    from formats.igz import load_into, save_scene
    assert Scene().dimension_style["norma"] == "iso"

    scene = Scene()
    scene.dimension_style["norma"] = "din"
    path = Path(tmp_path) / "cotas.igz"
    save_scene(scene, path)
    back = Scene()
    load_into(back, path)
    assert back.dimension_style["norma"] == "din"

    legacy = Scene()                       # the load path is an update()
    legacy.dimension_style.update({"decimals": 3, "units": "cm"})
    assert legacy.dimension_style["norma"] == "iso"


