# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Placed points and boxes moved to Rust
(``rust/ingetrazo_core/src/group_frame.rs``: a prototype's points through a
placement, and their box) and must answer as the Python did at
``dc7b485a``: ``Scene.bounds``, ``Scene.selection_bounds`` and
``core.group.placement_points``.

NumPy's ``(N, 3) @ (3, 3)`` runs through BLAS kernels that change with
``N`` (``tests/test_group_frame_parity.py``), so these agree to 4 ulp of
the coordinates' size, not bit for bit.
``tests/fixtures/bounds_golden.json.gz`` holds the answers; regenerate only
on purpose, against the reference implementation:
``python tests/test_bounds_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import math
import random
import sys
from pathlib import Path

import numpy as np

GOLDEN = Path(__file__).parent / "fixtures" / "bounds_golden.json.gz"


def _box_mesh(mesh, rng, n=2):
    from ingetrazo_core import Vec3 as V
    for _ in range(n):
        x, y, z = (rng.uniform(-5, 5) for _ in range(3))
        s = rng.uniform(0.2, 3)
        mesh.add_face([V(x, y, z), V(x + s, y, z), V(x + s, y + s, z), V(x, y + s, z)])
        mesh.add_face([V(x, y, z + s), V(x + s, y, z + s), V(x + s, y + s, z + s)])


def scenes():
    from ingetrazo_core import Matrix4 as M, Vec3 as V
    from core.group import Group
    from core.mesh import Mesh
    from core.scene import Scene
    rng = random.Random(20261124)
    out = []
    for k in range(24):
        scene = Scene()
        if k % 3:
            _box_mesh(scene.loose_mesh, rng, rng.randint(1, 3))
        far = (4.8e5, 4.6e6, 30.0) if k % 5 == 0 else (0.0, 0.0, 0.0)
        protos = []
        for _ in range(rng.randint(1, 4)):
            m = Mesh()
            _box_mesh(m, rng, rng.randint(1, 4))
            protos.append(m)
        for i in range(rng.randint(1, 8)):
            g = Group(rng.choice(protos) if rng.random() < 0.7 else Mesh(), f"g{i}")
            if not len(g.mesh.vertices):
                _box_mesh(g.mesh, rng, 1)
            if rng.random() < 0.85:
                x = M()
                x.translate(far[0] + rng.uniform(-50, 50), far[1] + rng.uniform(-50, 50),
                            far[2] + rng.uniform(-5, 5))
                x.rotate(rng.uniform(0, 360), rng.uniform(-1, 1), rng.uniform(-1, 1),
                         rng.uniform(0.1, 1))
                if rng.random() < 0.4:
                    x.scale(rng.uniform(0.5, 2), rng.uniform(0.5, 2), rng.choice((1.0, -1.0)))
                g.xform = x
            if rng.random() < 0.5:
                c = Group(rng.choice(protos), f"c{i}")
                y = M()
                y.translate(rng.uniform(-3, 3), rng.uniform(-3, 3), 0.0)
                y.rotate(rng.uniform(0, 360), 0.0, 0.0, 1.0)
                c.xform = y
                g.children.append(c)
            scene.groups.append(g)
        scene.version += 1
        out.append((scene, rng.random()))
    return out


def answer(scene, frac):
    from core.group import placement_points
    lo, hi = scene.bounds()
    pts = [placement_points(g).tolist() for g in scene.groups]
    n = max(1, int(len(scene.groups) * frac))
    scene.selection = set(scene.groups[:n]) | set(list(scene.loose_mesh.faces)[:1])
    slo, shi = scene.selection_bounds()
    tup = (lambda v: None if v is None else [v.x(), v.y(), v.z()])
    return {"bounds": [tup(lo), tup(hi)], "selection": [tup(slo), tup(shi)], "points": pts}


def _close(a, b):
    if a is None or b is None:
        return a is b
    if isinstance(a, list):
        return len(a) == len(b) and all(_close(x, y) for x, y in zip(a, b))
    return abs(a - b) <= 4 * math.ulp(max(abs(b), 1.0))


def test_boxes_and_placed_points_read_as_the_python_ones_did():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    cases = scenes()
    assert len(cases) == len(want)
    for i, ((scene, frac), w) in enumerate(zip(cases, want)):
        g = answer(scene, frac)
        for key in w:
            assert _close(g[key], w[key]), (i, key)


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    rows = [answer(s, f) for s, f in scenes()]
    GOLDEN.write_bytes(gzip.compress(json.dumps(rows, separators=(",", ":")).encode(),
                                     mtime=0))
    print(len(rows), "scenes ->", GOLDEN, GOLDEN.stat().st_size)
