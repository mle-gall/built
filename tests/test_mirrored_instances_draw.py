# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A mirrored component placement draws instanced and shows its FRONT
(issue #158).

Mirrored placements used to leave the instanced pass — the mirror flips
the winding of every prototype triangle and GL would call the painted side
the back — and fell back to one baked world copy each. An industrial model
from @pacaeiro had 6 203 of them: 4.2 million faces baked one by one on the
first frame. They draw instanced now, in batches under a clockwise front
face. This renders both placements and reads the pixels: each shows its
front colour, not the back tint.

Needs a GL context that renders; skipped where the platform has none."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V
from PySide6.QtWidgets import QApplication

_app = QApplication.instance() or QApplication([])

AMARILLO = (1.0, 0.85, 0.0)


def test_a_mirrored_placement_draws_instanced_with_its_front_colour():
    from core.group import Group
    from core.mesh import Mesh
    from views.main_window import MainWindow
    win = MainWindow()
    try:
        win.show()
        for _ in range(10):
            _app.processEvents()
        vp = win.viewport
        if getattr(vp, "_gl", None) is None or not vp.isValid():
            pytest.skip("no OpenGL context on this platform")
        sc = vp.scene
        sc.guides.clear()
        proto = Mesh()
        f = proto.add_face([V(-1, -1, 0), V(1, -1, 0), V(1, 1, 0), V(-1, 1, 0)])
        if f.normal().z() < 0:
            f.flip()
        f.attrs["color"] = list(AMARILLO)
        derecha, espejo = QMatrix4x4(), QMatrix4x4()
        derecha.translate(-2.0, 0.0, 0.0)
        espejo.translate(2.0, 0.0, 0.0)
        espejo.scale(-1.0, 1.0, 1.0)
        for m in (derecha, espejo):
            g = Group(proto, name="Pieza")
            g.xform = m
            sc.groups.append(g)
        sc.version += 1
        vp.camera.set_view("top")                     # looking down -Z: the front
        vp.camera.perspective = False
        vp.camera.fit_to(V(-4, -2, 0), V(4, 2, 0))
        for _ in range(5):
            _app.processEvents()
        img = vp.grabFramebuffer()
        lotes = getattr(vp, "_frame_instanced", {}) or {}
        assert any(k[2] for k in lotes), "the mirrored placement is instanced"
        assert not any(id(g) in getattr(vp, "_inst_chunks", {}) for g in sc.groups), \
            "and no placement was baked to world coordinates"
        dpr = img.width() / max(vp.width(), 1)
        for x in (-2.0, 2.0):
            px = vp._world_to_pixel(V(x, 0.5, 0))     # off the red axis
            c = img.pixelColor(int(px[0] * dpr), int(px[1] * dpr))
            r, g, b = c.redF(), c.greenF(), c.blueF()
            assert r > 0.6 and g > 0.5 and b < 0.35, (x, (r, g, b))
    finally:
        win._saved_version = win.viewport.scene.version
        win.close()


def _prisma(n=12, r=1.0, h=2.0):
    """A smooth n-sided prism: soft vertical edges, a curved side."""
    import math
    from core.mesh import Mesh
    m = Mesh()
    ring = [(r * math.cos(2 * math.pi * i / n), r * math.sin(2 * math.pi * i / n))
            for i in range(n)]
    for i in range(n):
        (x0, y0), (x1, y1) = ring[i], ring[(i + 1) % n]
        m.add_face([V(x0, y0, 0), V(x1, y1, 0), V(x1, y1, h), V(x0, y0, h)])
    for e in m.edges:
        if abs(e.a.z() - e.b.z()) > 1e-9:
            e.soft = True
    return m


def test_the_placements_signature_survives_an_orbit_and_follows_an_edit():
    """Orbiting a big model walked every placement on every frame to sign
    them (#158). The signature is kept while the scene version stands and
    re-signed when a command moves a placement (and bumps the version)."""
    from core.group import Group
    from views.main_window import MainWindow
    win = MainWindow()
    try:
        vp = win.viewport
        g = Group(_prisma(), name="P")
        g.xform = QMatrix4x4()
        vp.scene.groups.append(g)
        vp.scene.version += 1
        e0 = vp._placements_epoch()
        vp._tick = getattr(vp, "_tick", 0) + 1         # next frame
        vp.camera.orbit(6.0, 2.0, 600)
        assert vp._placements_epoch() == e0
        assert vp._placements() is vp._placements()
        m = QMatrix4x4()
        m.translate(1.0, 0.0, 0.0)
        g.xform = m * g.xform
        vp.scene.version += 1                           # what every tool does
        vp._tick += 1
        assert vp._placements_epoch() != e0
    finally:
        win._saved_version = win.viewport.scene.version
        win.close()
