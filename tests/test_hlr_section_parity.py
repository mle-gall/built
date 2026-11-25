# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The section cut's maths moved to Rust (``rust/built_core/src/hlr.rs``:
``clip_to_section``, ``section_loops``) and must answer bit for bit as
``core/hlr.py``'s Python did at ``74b54c81``.

``tests/fixtures/hlr_section_golden.json.gz`` holds those answers on seeded
boxes, prisms and loose triangles cut by seeded planes, with the chords
chained into rings (default and given tolerances, the rows each ring came
from). Regenerate only on purpose, against the reference implementation:
``python tests/test_hlr_section_parity.py --write``."""
from __future__ import annotations

from tests.platform_golden import matches
import built_core
import gzip
import json
import math
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "hlr_section_golden.json.gz"


def _box(x0, y0, z0, sx, sy, sz):
    p = [(x0 + dx * sx, y0 + dy * sy, z0 + dz * sz)
         for dz in (0, 1) for dy in (0, 1) for dx in (0, 1)]
    quads = [(0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4), (2, 6, 7, 3),
             (0, 4, 6, 2), (1, 3, 7, 5)]
    tris = []
    for a, b, c, d in quads:
        tris.append((p[a], p[b], p[c]))
        tris.append((p[a], p[c], p[d]))
    hard = [(p[a], p[b]) for a, b in ((0, 1), (1, 3), (3, 2), (2, 0), (4, 5), (5, 7),
                                      (7, 6), (6, 4), (0, 4), (1, 5), (2, 6), (3, 7))]
    return tris, hard


def _prism(rng, cx, cy, n, r, h):
    from tests.unit_rings import rings
    ring = [(cx + r * x, cy + r * y) for x, y in rings[n]]
    tris, hard = [], []
    for k in range(n):
        (ax, ay), (bx, by) = ring[k], ring[(k + 1) % n]
        a0, b0, a1, b1 = (ax, ay, 0.0), (bx, by, 0.0), (ax, ay, h), (bx, by, h)
        tris += [(a0, b0, b1), (a0, b1, a1), ((cx, cy, 0.0), b0, a0), ((cx, cy, h), a1, b1)]
        hard += [(a0, b0), (a1, b1), (a0, a1)]
    return tris, hard


class _Plane:
    def __init__(self, point, normal):
        from built_core import Vec3 as V
        self.point, self.normal = V(*point), V(*normal)


def cases():
    rng = random.Random(20261124)
    out = []
    for k in range(60):
        tris, hard = [], []
        for _ in range(rng.randint(1, 5)):
            if rng.random() < 0.6:
                t, h = _box(rng.uniform(-5, 5), rng.uniform(-5, 5), rng.uniform(-1, 1),
                            rng.uniform(0.1, 3), rng.uniform(0.1, 3), rng.uniform(0.5, 4))
            else:
                t, h = _prism(rng, rng.uniform(-5, 5), rng.uniform(-5, 5), rng.randint(3, 24),
                              rng.uniform(0.2, 2), rng.uniform(0.5, 3))
            tris += t
            hard += h
        for _ in range(rng.randint(0, 6)):
            tris.append(tuple(tuple(rng.uniform(-6, 6) for _ in range(3)) for _ in range(3)))
        soft = [(a, b, (0.0, 0.0, 1.0), None if rng.random() < 0.2 else (1.0, 0.0, 0.0))
                for a, b in hard[: len(hard) // 3]]
        if k % 3 == 0:
            normal = (0.0, 0.0, 1.0)
        elif k % 3 == 1:
            normal = (rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-1, 1))
            ln = math.sqrt(sum(v * v for v in normal)) or 1.0
            normal = tuple(v / ln for v in normal)
        else:
            normal = (1.0, 0.0, 0.0)
        point = (rng.uniform(-2, 2), rng.uniform(-2, 2), rng.choice((0.0, 1.0, rng.uniform(0, 3))))
        out.append((tris, hard, soft, point, normal))
    return out


def answer(hlr, case):
    tris, hard, soft, point, normal = case
    plane = _Plane(point, normal)
    t, h, s = hlr.clip_to_section(tris, hard, soft, plane)
    t2, h2, s2, cuts = hlr.clip_to_section(tris, hard, soft, plane, split_cuts=True)
    members: list = []
    loops = hlr.section_loops(cuts, members=members)
    loops_tol = hlr.section_loops(cuts, tol=1e-3)
    return {"tris": t, "hard": h, "soft": s, "split": [t2, h2, s2, cuts],
            "loops": [r.tolist() for r in loops], "members": members,
            "loops_tol": [r.tolist() for r in loops_tol],
            "tags": built_core.hlr_closing_tags(cuts, [i % 3 for i in range(len(cuts))])}


def _jsonable(x):
    if isinstance(x, float) and not math.isfinite(x):
        return repr(x)
    if isinstance(x, (list, tuple)):
        return [_jsonable(v) for v in x]
    if isinstance(x, dict):
        return {k: _jsonable(v) for k, v in x.items()}
    return x


def test_the_section_maths_answers_as_the_python_one_did():
    from built_core import hlr
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = [_jsonable(answer(hlr, c)) for c in cases()]
    assert len(got) == len(want)
    wrong = [f"case {i} {k}" for i, (g, w) in enumerate(zip(got, want))
             for k in w if not matches(g[k], w[k])]
    assert not wrong, f"{len(wrong)} answers differ: {wrong[:20]}"


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from built_core import hlr as _hlr
    rows = [_jsonable(answer(_hlr, c)) for c in cases()]
    GOLDEN.write_bytes(gzip.compress(json.dumps(rows, separators=(",", ":")).encode(),
                                     mtime=0))
    print(len(rows), "cases ->", GOLDEN, GOLDEN.stat().st_size)
