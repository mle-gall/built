# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The section cut's geometry for the viewport moved to Rust
(``rust/ingetrazo_core/src/model/section_fill.rs``: the plane's reach and
chords, the filled areas, their triangles and hatching, the sheet basis) and
must answer as ``core/section.py``'s NumPy did at ``9cf53080``.

``tests/fixtures/section_fill_golden.json.gz`` holds those answers on seeded
boxes (some hollow, some crossing), prisms and loose triangles, cut by seeded
planes, with and without a visibility mask. Chords, triangles and hatch lines
are float32 and compared bit for bit. Regenerate only on purpose, against the
reference implementation: ``python tests/test_section_fill_parity.py --write``."""
from __future__ import annotations

import ingetrazo_core
import gzip
import json
import math
import random
import sys
from pathlib import Path

import numpy as np

GOLDEN = Path(__file__).parent / "fixtures" / "section_fill_golden.json.gz"


def _box(x0, y0, z0, sx, sy, sz, flip=False):
    p = [(x0 + dx * sx, y0 + dy * sy, z0 + dz * sz)
         for dz in (0, 1) for dy in (0, 1) for dx in (0, 1)]
    quads = [(0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4), (2, 6, 7, 3),
             (0, 4, 6, 2), (1, 3, 7, 5)]
    tris = []
    for a, b, c, d in quads:
        if flip:
            a, b, c, d = d, c, b, a
        tris.append((p[a], p[b], p[c]))
        tris.append((p[a], p[c], p[d]))
    return tris


def _prism(cx, cy, n, r, h):
    ring = [(cx + r * math.cos(2 * math.pi * k / n), cy + r * math.sin(2 * math.pi * k / n))
            for k in range(n)]
    tris = []
    for k in range(n):
        (ax, ay), (bx, by) = ring[k], ring[(k + 1) % n]
        a0, b0, a1, b1 = (ax, ay, 0.0), (bx, by, 0.0), (ax, ay, h), (bx, by, h)
        tris += [(a0, b0, b1), (a0, b1, a1), ((cx, cy, 0.0), b0, a0), ((cx, cy, h), a1, b1)]
    return tris


def cases():
    rng = random.Random(20261124)
    out = []
    for k in range(96):
        tris = []
        for _ in range(rng.randint(1, 4)):
            r = rng.random()
            if r < 0.35:
                tris += _box(rng.uniform(-5, 5), rng.uniform(-5, 5), rng.uniform(-1, 1),
                             rng.uniform(0.1, 3), rng.uniform(0.1, 3), rng.uniform(0.5, 4))
            elif r < 0.6:
                # A hollow box: a smaller one inside, and maybe an island in it.
                x, y, sx, sy = rng.uniform(-5, 5), rng.uniform(-5, 5), rng.uniform(2, 4), rng.uniform(2, 4)
                tris += _box(x, y, -1.0, sx, sy, 4.0)
                tris += _box(x + 0.3, y + 0.3, -0.5, sx - 0.6, sy - 0.6, 3.0, flip=True)
                if rng.random() < 0.5:
                    tris += _box(x + 0.8, y + 0.8, -0.2, 0.4, 0.4, 2.0)
            else:
                tris += _prism(rng.uniform(-5, 5), rng.uniform(-5, 5), rng.randint(3, 24),
                               rng.uniform(0.2, 2), rng.uniform(0.5, 3))
        for _ in range(rng.randint(0, 6)):
            tris.append(tuple(tuple(rng.uniform(-6, 6) for _ in range(3)) for _ in range(3)))
        if k % 4 == 0:
            normal = (0.0, 0.0, 1.0)
        elif k % 4 == 1:
            normal = (rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-1, 1))
        elif k % 4 == 2:
            normal = (1.0, 0.0, 0.0)
        else:
            normal = (rng.uniform(-0.2, 0.2), rng.uniform(-1, 1), rng.uniform(-0.3, 0.3))
        point = (rng.uniform(-2, 2), rng.uniform(-2, 2), rng.choice((0.0, 1.0, rng.uniform(0, 3))))
        visible = [rng.random() > 0.1 for _ in tris] if k % 5 == 3 else None
        spacing = rng.choice((0.05, 0.1, 0.25, 0.4))
        out.append((tris, normal, point, visible, spacing))
    return out


def _arr(a):
    if a is None:
        return None
    a = np.asarray(a)
    return [str(a.dtype), list(a.shape), a.astype(np.float64).ravel().tolist()
            if a.dtype.kind == "f" else a.ravel().tolist()]


def answer(section, case):
    from ingetrazo_core import Vec3 as V
    tris, normal, point, visible, spacing = case
    t = np.asarray(tris, dtype=np.float64)
    v0 = t[:, 0].copy()
    e1 = t[:, 1] - v0
    e2 = t[:, 2] - v0
    n = V(*normal)
    if n.length() > 0:
        n = n.normalized()
    p = V(*point)
    vis = None if visible is None else np.asarray(visible)
    chords = ingetrazo_core.section_cut_chords(v0, e1, e2, n, p, vis)
    rows = ingetrazo_core.section_cut_chords(v0, e1, e2, n, p, vis, rows=True)
    reach = ingetrazo_core.PlaneReach(v0, e1, e2, n)
    slid = [reach.cut(p + n * dz, vis, rows=True) for dz in (-0.5, 0.25)]
    got = {"chords": _arr(chords), "rows": [_arr(rows[0]), _arr(rows[1])],
           "slid": [[_arr(a), _arr(b)] for a, b in slid]}
    got["fill"] = _arr(ingetrazo_core.section_fill_triangles(chords, n))
    got["hatch"] = _arr(ingetrazo_core.section_hatch_segments(chords, n, spacing))
    got["cross"] = _arr(ingetrazo_core.section_hatch_segments(chords, n, spacing * 1.5, cross=True))
    u, v = ingetrazo_core.section_sheet_basis(n)
    got["sheet"] = [list(map(float, u)), list(map(float, v))]
    return got


def _jsonable(x):
    if isinstance(x, float) and not math.isfinite(x):
        return repr(x)
    if isinstance(x, (list, tuple)):
        return [_jsonable(v) for v in x]
    if isinstance(x, dict):
        return {k: _jsonable(v) for k, v in x.items()}
    return x


def test_the_section_fill_answers_as_the_numpy_one_did():
    from core import section
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = [_jsonable(answer(section, c)) for c in cases()]
    assert len(got) == len(want)
    wrong = [f"case {i} {k}" for i, (g, w) in enumerate(zip(got, want))
             for k in w if g[k] != w[k]]
    assert not wrong, f"{len(wrong)} answers differ: {wrong[:20]}"


def test_the_corpus_has_holes_and_hatching():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    assert sum(1 for w in want if w["fill"] and w["fill"][1][0]) > 40
    assert sum(1 for w in want if w["hatch"] and w["hatch"][1][0]) > 40
    assert sum(1 for w in want if w["chords"] is None) >= 1


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from core import section
    data = [_jsonable(answer(section, c)) for c in cases()]
    GOLDEN.parent.mkdir(exist_ok=True)
    GOLDEN.write_bytes(gzip.compress(json.dumps(data).encode(), mtime=0))
    print(f"wrote {GOLDEN} ({len(data)} cases)")
