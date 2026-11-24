# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Issue #432 (@dns-0): Entity Info wrote a face's area in m² whatever the
document's units — a feet-and-inches model showed a metric area."""
from __future__ import annotations

from ingetrazo_core import Vec3 as V
from core import units
from tests.native_tree import inspector
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def _face_text(host, length_unit):
    scene = host.viewport.scene
    scene.units = {"length": length_unit, "precision": 2}
    face = scene.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    return inspector(host, [face])["Geometry"]["Area"]


def test_a_face_in_a_feet_document_shows_square_feet(native_host):
    text = _face_text(native_host, "ft")
    assert "m²" not in text
    assert text == units.fmt_area_with(1.0, native_host.viewport.scene.units)


def test_a_face_in_a_metre_document_still_shows_square_metres(native_host):
    assert "m²" in _face_text(native_host, "m")
