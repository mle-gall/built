# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The STL export writes the unit the user picks, millimetres first (#513).

An STL records no unit and slicers read it as millimetres. The exporter
wrote the model's own metres, so a 15 x 25 mm part came into Simplify3D
1000x too small. ``save_stl`` takes the unit now (metres by default, for
callers that want the raw model), and Export ▸ STL asks, like the import.
"""
from __future__ import annotations

from tests.shell_isolation import one_off_export, restore_shell_patches  # noqa: F401
import os
import struct

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest  # noqa: E402
from ingetrazo_core import Vec3 as V

from core.scene import Scene  # noqa: E402
from formats import stl as stl_format  # noqa: E402


def _part():
    """The report's part: a 15 x 25 mm plate."""
    sc = Scene()
    sc.mesh.add_face([V(0, 0, 0), V(0.015, 0, 0),
                      V(0.015, 0.025, 0), V(0, 0.025, 0)])
    return sc


def _extent(path):
    data = path.read_bytes()
    (n,) = struct.unpack_from("<I", data, 80)
    xs, ys = [], []
    for i in range(n):
        vals = struct.unpack_from("<12f", data, 84 + 50 * i)
        for k in (3, 6, 9):
            xs.append(vals[k]); ys.append(vals[k + 1])
    return max(xs) - min(xs), max(ys) - min(ys)


@pytest.mark.parametrize("unit, size", [
    ("mm", (15.0, 25.0)), ("cm", (1.5, 2.5)), ("m", (0.015, 0.025)),
])
def test_the_file_is_written_in_the_chosen_unit(tmp_path, unit, size):
    out = tmp_path / f"part-{unit}.stl"
    stl_format.save_stl(_part(), out, unit)
    w, h = _extent(out)
    assert w == pytest.approx(size[0], rel=1e-5)
    assert h == pytest.approx(size[1], rel=1e-5)


def test_without_a_unit_it_stays_in_metres(tmp_path):
    out = tmp_path / "part.stl"
    stl_format.save_stl(_part(), out)
    assert _extent(out) == pytest.approx((0.015, 0.025), rel=1e-5)


def test_exported_in_mm_and_imported_as_mm_keeps_the_size(tmp_path):
    out = tmp_path / "part.stl"
    stl_format.save_stl(_part(), out, "mm")
    mesh = stl_format.parse_stl(out, scale=stl_format.STL_UNITS["mm"])
    pts = [p for f in mesh.faces for p in f.vertices]
    w = max(p.x() for p in pts) - min(p.x() for p in pts)
    assert w == pytest.approx(0.015, rel=1e-5)


@pytest.mark.parametrize(("last_unit", "expected", "size"), [
    (None, "mm", (15.0, 25.0)), ("incompatible", "mm", (15.0, 25.0)),
    ("cm", "cm", (1.5, 2.5)),
])
def test_stl_sheet_remembers_the_unit(native_host, tmp_path, last_unit, expected, size):
    from core.settings import settings

    out = tmp_path / "part.stl"
    settings().remove("export/stl_unit")
    if last_unit is not None:
        settings().setValue("export/stl_unit", last_unit)
    win = native_host.window
    win.viewport.scene.mesh.add_face(
        [V(0, 0, 0), V(0.015, 0, 0), V(0.015, 0.025, 0), V(0, 0.025, 0)])
    win.viewport.scene.version += 1
    form = one_off_export(win, "stl", out)
    assert form.value("unit") == expected
    assert _extent(out) == pytest.approx(size, rel=1e-5)
    assert settings().value("export/stl_unit") == expected
