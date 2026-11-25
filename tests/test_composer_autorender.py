# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Composer auto-render (the usual «Auto» for sheets) and the array geometry
path the hidden-line pass takes from the viewport caches."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
import math

import numpy as np
from built_core import Vec3 as QVector3D


def _fountain_like(scene):
    """A soft-edged solid of revolution in a group + a loose square: the
    two families collect_geometry walks."""
    from core.ai import _recipe_helpers
    helpers = _recipe_helpers(scene)
    helpers["revolve"]([(0.5, 0.0), (0.6, 0.5), (0.4, 1.0)], segments=24,
                       name="Vaso")
    scene.mesh.add_face([QVector3D(-2, -2, 0), QVector3D(2, -2, 0),
                         QVector3D(2, 2, 0), QVector3D(-2, 2, 0)])
    scene.version += 1


def test_hlr_geometry_matches_the_python_walk(native_host):
    """Viewport.hlr_geometry() (arrays from the caches) carries the same
    triangles, hard edges and soft edges as core.hlr.collect_geometry, and
    hlr_view draws the same segments from either."""
    from core.camera import OrbitCamera
    from core.hlr import collect_geometry, hlr_view

    win = native_host.window
    vp = win.viewport
    native_host.resize(800, 600, 1.)
    scene = vp.scene
    _fountain_like(scene)
    tris, hard, soft, soft_n = vp.hlr_geometry()
    lt, lh, ls = collect_geometry(scene)
    assert (len(tris), len(hard), len(soft)) == (len(lt), len(lh), len(ls))
    assert soft_n.shape == (len(soft), 2, 3)

    def sym(pairs):
        out = set()
        for a, b in pairs:
            a = tuple(np.round(np.asarray(a, dtype=float), 5))
            b = tuple(np.round(np.asarray(b, dtype=float), 5))
            out.add((a, b) if a <= b else (b, a))
        return out
    assert sym(hard) == sym(lh)
    assert sym(soft) == sym([(p0, p1) for p0, p1, _a, _b in ls])
    # Open-boundary flag: the revolve caps close everything here.
    assert not np.isnan(soft_n[:, 1, 0]).any()

    cam = OrbitCamera()
    cam.set_view("front")
    cam.perspective = False
    lo, hi = scene.bounds()
    cam.fit_to(lo, hi, 1.1)
    slow = hlr_view(scene, cam)
    fast = hlr_view(scene, cam, geometry=(tris, hard, soft, soft_n))
    assert len(fast) == len(slow)
    assert sym([(s[:2], s[2:]) for s in slow]) == sym(
        [(s[:2], s[2:]) for s in fast])
    # Section cut: the array path falls back to the list clipper.
    from built_core.section import SectionPlane
    sp = SectionPlane(QVector3D(0, 0, 0.5), QVector3D(0, 0, 1),
                      name="Planta", symbol="1")
    scene.section_planes.append(sp)
    scene.set_active_section(sp)
    cut_slow = hlr_view(scene, cam)
    cut_fast = hlr_view(scene, cam, geometry=(tris, hard, soft, soft_n))
    assert len(cut_fast) == len(cut_slow) != len(slow)   # chords added


