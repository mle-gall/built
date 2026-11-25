# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A group's derived frame and oriented box moved to Rust
(``rust/built_core/src/group_frame.rs``) and must answer as
``core/group.py``'s Python did at ``74b54c81``.

- ``frame_from_points`` (and its convex hull) bit for bit: it picks the
  smallest of tied areas on symmetric shapes (a square, a circle), so a
  last-bit difference would turn the frame. NumPy's ``hull @ u`` there is
  the plain left-to-right sum (measured), which the port does.
- ``oriented_bounds`` within a few ulps: NumPy's ``(N, 3) @ (3, 3)`` runs
  through BLAS kernels that change with ``N`` (a fused multiply-add chain
  for most sizes, other orders for a few), so no single order is NumPy's
  for every input. The port uses the fused chain; the bounds agree to
  ``4 ulp`` of the extent.

``tests/fixtures/group_frame_golden.json.gz`` holds the answers;
regenerate only on purpose, against the reference implementation:
``python tests/test_group_frame_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import math
import sys
from pathlib import Path

import numpy as np

GOLDEN = Path(__file__).parent / "fixtures" / "group_frame_golden.json.gz"


def point_sets():
    rng = np.random.default_rng(20261124)
    out = [np.zeros((0, 3)), np.zeros((2, 3)), np.array([[0, 0, 0], [1, 0, 0], [2, 0, 0.0]]),
           np.array([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0.0]]),        # a square: ties
           np.array([[0, 0, 0], [0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 1, 0.0]])]
    for k in range(70):
        kind = k % 7
        n = int(rng.integers(3, 400))
        if kind == 0:      # a rotated rectangle's corners and edge points
            th = rng.uniform(0, math.pi)
            w, h = rng.uniform(0.5, 5, 2)
            t = rng.uniform(0, 1, n)
            side = rng.integers(0, 4, n)
            x = np.where(side < 2, t * w, np.where(side == 2, 0, w))
            y = np.where(side >= 2, t * h, np.where(side == 0, 0, h))
            pts = np.column_stack([x * math.cos(th) - y * math.sin(th),
                                   x * math.sin(th) + y * math.cos(th), rng.uniform(0, 3, n)])
        elif kind == 1:    # a regular polygon: every edge ties
            m = int(rng.integers(3, 33))
            a = np.arange(m) * 2 * math.pi / m + rng.uniform(0, 1)
            pts = np.column_stack([np.cos(a), np.sin(a), np.zeros(m)]) * rng.uniform(0.2, 9)
        elif kind == 2:    # a cloud
            pts = rng.normal(0, rng.uniform(0.1, 20), (n, 3))
        elif kind == 3:    # survey coordinates
            pts = rng.uniform(-30, 30, (n, 3)) + [4.8e5, 4.6e6, 30.0]
        elif kind == 4:    # an axis-aligned box's vertices, repeated
            pts = np.array([[x, y, z] for x in (0, 2.0) for y in (0, 1.0) for z in (0, 3.0)] * 3)
        elif kind == 5:    # integer lattice: many collinear points
            pts = rng.integers(-5, 6, (n, 3)).astype(float)
        else:              # a big cloud (prunes twice before the scan)
            pts = rng.uniform(-50, 50, (int(rng.integers(500, 5000)), 3))
        out.append(np.asarray(pts, float))
    return out


def answer(group, pts):
    frame = group.frame_from_points(pts if len(pts) else None)
    hull = group._hull_2d(pts[:, :2]) if len(pts) else np.zeros((0, 2))
    fr, lo, hi = group.oriented_bounds(None, frame, pts)
    return {"frame": [list(v.toTuple()) for v in frame], "hull": np.asarray(hull).tolist(),
            "lo": list(map(float, lo)), "hi": list(map(float, hi))}


def test_the_frame_is_the_python_one_bit_for_bit_and_the_box_within_ulps():
    from core import group
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    sets = point_sets()
    assert len(sets) == len(want)
    for i, (pts, w) in enumerate(zip(sets, want)):
        g = answer(group, pts)
        assert g["frame"] == w["frame"], i
        assert g["hull"] == w["hull"], i
        extent = max(1.0, *map(abs, w["lo"] + w["hi"]))
        tol = 4 * math.ulp(extent)
        for k in ("lo", "hi"):
            assert all(abs(a - b) <= tol for a, b in zip(g[k], w[k])), (i, k, g[k], w[k])


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from core import group as _group
    rows = [answer(_group, p) for p in point_sets()]
    GOLDEN.write_bytes(gzip.compress(json.dumps(rows, separators=(",", ":")).encode(),
                                     mtime=0))
    print(len(rows), "point sets ->", GOLDEN, GOLDEN.stat().st_size)
