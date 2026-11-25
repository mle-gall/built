# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Rust engine's ear clipping gives the Python port's triangles.

Every face of a model is triangulated for the picture, the pick index,
the hidden lines and the section fills — 90 000 of them in Python ear by
ear when the plaza opens or is drawn by layer (3.8 s of its 16 s of
lines). The engine's `earcut` is the same Mapbox port step for step, so
the triangles are the same, index for index, in the same order: a
different valid triangulation would still move the seams of a textured
face and the pixels of every picture test."""
from __future__ import annotations

import hashlib
import math
import random

import numpy as np
from built_core import Vec3

from built_core import triangulate as T


def _record(digest, points, holes):
    triangles = T.earcut(points, holes)
    for triangle in triangles:
        digest.update(bytes(triangle))
    digest.update(b"\xff")
    return len(triangles)


def _star(rng, n, r0=1.0, r1=3.0, cx=0.0, cy=0.0):
    a = np.sort(rng.uniform(0, 2 * np.pi, n))
    r = rng.uniform(r0, r1, n)
    return np.column_stack([cx + r * np.cos(a), cy + r * np.sin(a)])


def test_random_polygons_with_holes_give_the_same_triangles():
    rng = np.random.default_rng(20261111)
    digest = hashlib.sha256()
    seen = 0
    for _ in range(300):
        outer = _star(rng, int(rng.integers(3, 40)))
        pts = [outer]
        holes = []
        for _h in range(int(rng.integers(0, 4))):
            at = len(np.concatenate(pts))
            holes.append(at)
            c = rng.uniform(-0.6, 0.6, 2)
            pts.append(_star(rng, int(rng.integers(3, 9)), 0.1, 0.4, *c)[::-1])
        points = np.concatenate(pts)
        seen += _record(digest, points, holes)
    assert seen > 5000
    assert digest.hexdigest() == "c2864355b4a028b07c602aeb63d00f2c3df805743a45dbb672b48e4eeb80c338"


def test_the_degenerate_cases_recover_the_same_way():
    cases = [
        # collinear points on an edge, duplicate corners
        ([(0, 0), (1, 0), (2, 0), (2, 2), (2, 2), (0, 2), (0, 1)], []),
        # a square with a hole touching its edge, and a one-point hole
        ([(0, 0), (4, 0), (4, 4), (0, 4), (0, 2), (1, 2), (1, 3), (0, 3),
          (2, 2)], [4, 8]),
        # two holes at the same height, the bridge's tie
        ([(0, 0), (6, 0), (6, 4), (0, 4), (1, 1), (1, 3), (2, 3), (2, 1),
          (4, 1), (4, 3), (5, 3), (5, 1)], [4, 8]),
        # a self-touching outline (a figure of eight)
        ([(0, 0), (2, 0), (2, 2), (1, 1), (0, 2), (1, 1)], []),
        # a slit: an outline that folds back on itself
        ([(0, 0), (4, 0), (4, 1), (1, 1), (1, 1.0000001), (4, 1.0000001),
          (4, 2), (0, 2)], []),
        # fewer than three points, and all collinear
        ([(0, 0), (1, 0)], []),
        ([(0, 0), (1, 0), (2, 0), (3, 0)], []),
        # clockwise outer, counter-clockwise hole (both turned round)
        ([(0, 0), (0, 4), (4, 4), (4, 0), (1, 1), (3, 1), (3, 3), (1, 3)], [4]),
    ]
    digest = hashlib.sha256()
    for points, holes in cases:
        _record(digest, points, holes)
    assert digest.hexdigest() == "f0a4116822a9f680ea96ad9d100821b198c527efe56187d99f47e54d7d0682c9"


def test_projected_vertex_identity_and_order():
    # Fix the input directions as binary64: a libm ulp in a nearly coplanar
    # point can change the selected diagonal before earcut is even called.
    from tests.unit_rings import rings
    rng = random.Random(372901)
    digest = hashlib.sha256()
    for k in range(500):
        size = rng.randrange(3, 65)
        radii = [rng.uniform(2., 7.) for _ in range(size)]
        tilt = rng.uniform(-2., 2.)

        def point(x, y):
            return Vec3(600000. + x, 6800000. + y, tilt * x + y)

        outer = [point(r * x, r * y) for r, (x, y) in zip(radii, rings[size])]
        hole = [point(-1, -1), point(-1, 1), point(1, 1), point(1, -1)] if k % 2 else []
        indices = {id(p): i for i, p in enumerate(outer + hole)}
        for tri in T.triangulate(outer, [hole]):
            digest.update(bytes(indices[id(p)] for p in tri))
        digest.update(b'\xff')
    assert digest.hexdigest() == "d0d073e0c5bd3c2c1cde744f4416a5e93cbedcdf907776c0eb5c9759b78b6389"
