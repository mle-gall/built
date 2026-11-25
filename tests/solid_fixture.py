# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic solid geometry for the plugin and the native Problems inspector."""
from built_core import Vec3


def _box(mesh, x0=0.0, y0=0.0, z0=0.0, dx=2.0, dy=2.0, dz=2.0,
         skip_top=False):
    """An axis-aligned box with consistent outward windings (the floor is
    clockwise seen from above — the showcase-script lesson)."""
    P = Vec3
    x1, y1, z1 = x0 + dx, y0 + dy, z0 + dz
    quads = [
        [P(x0, y1, z0), P(x1, y1, z0), P(x1, y0, z0), P(x0, y0, z0)],
        [P(x0, y0, z0), P(x1, y0, z0), P(x1, y0, z1), P(x0, y0, z1)],
        [P(x1, y0, z0), P(x1, y1, z0), P(x1, y1, z1), P(x1, y0, z1)],
        [P(x1, y1, z0), P(x0, y1, z0), P(x0, y1, z1), P(x1, y1, z1)],
        [P(x0, y1, z0), P(x0, y0, z0), P(x0, y0, z1), P(x0, y1, z1)],
    ]
    if not skip_top:
        quads.append([P(x0, y0, z1), P(x1, y0, z1), P(x1, y1, z1),
                      P(x0, y1, z1)])
    return [mesh.add_face(q) for q in quads]


def _top(mesh, z=2.0):
    return next(f for f in mesh.faces
                if all(abs(v.position.z() - z) < 1e-6 for v in f.loop))
