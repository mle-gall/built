# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A dimension's frame maths moved to Rust
(``rust/built_core/src/model/dimension_frame.rs``) and must answer bit for
bit as ``core/dimension.py``'s Python did at ``9cf53080``: the linear frame,
the measured value, the squared offset, the dimension line's ends and
midpoint, the cursor's offset and the kind of dimension it asks for.

``tests/fixtures/dimension_frame_golden.json.gz`` holds those answers on
seeded segments (axis-aligned, slanted, degenerate, near-parallel to an axis)
with seeded offsets and cursors. Regenerate only on purpose, against the
reference implementation: ``python tests/test_dimension_frame_parity.py --write``."""
from __future__ import annotations

import built_core
import gzip
import json
import math
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "dimension_frame_golden.json.gz"


def cases():
    rng = random.Random(20261124)
    out = []
    for k in range(400):
        a = [rng.uniform(-20, 20) for _ in range(3)]
        kind = k % 8
        if kind == 0:
            b = list(a)
            b[rng.randrange(3)] += rng.uniform(-5, 5)          # along an axis
        elif kind == 1:
            b = [c + rng.uniform(-1e-10, 1e-10) for c in a]    # degenerate
        elif kind == 2:
            b = [a[0] + rng.uniform(-5, 5), a[1] + rng.uniform(-5, 5), a[2]]   # in plan
        elif kind == 3:
            t = math.radians(rng.choice((14.9, 15.0, 15.1, 74.9, 75.0, 75.1)))
            b = [a[0] + 3 * math.cos(t), a[1], a[2] + 3 * math.sin(t)]
        else:
            b = [c + rng.uniform(-5, 5) for c in a]
        off = [rng.uniform(-3, 3) for _ in range(3)]
        if kind == 4:
            off = [0.0, 0.0, 0.0]
        cursors = [[c + rng.uniform(-8, 8) for c in a] for _ in range(4)]
        mid = [(x + y) / 2 for x, y in zip(a, b)]
        cursors.append([m + rng.uniform(-0.01, 0.01) for m in mid])
        out.append((a, b, off, cursors))
    return out


def _v(p):
    return [p.x(), p.y(), p.z()]


def answer(dimension, case):
    from built_core import Vec3 as V
    a, b, off, cursors = case
    A, B = V(*a), V(*b)
    got = {}
    for axis in (None, "x", "y", "z", "w"):
        d = dimension.Dimension(A, B, V(*off), axis=axis)
        frame = built_core.dimension_linear_frame(A, B, d.axis)
        lp = d.line_points()
        got[str(axis)] = [None if frame is None else [_v(frame[0]), _v(frame[1])],
                          d.value(), _v(d.perpendicular_offset()), [_v(lp[0]), _v(lp[1])],
                          _v(d.midpoint()), _v(d.line_point_a), _v(d.line_point_b), d.axis]
    got["cursor"] = [[_v(dimension.Dimension.offset_for_cursor(A, B, V(*c))),
                      (lambda o, ax: [_v(o), ax])(*dimension.Dimension.placement_for_cursor(
                          A, B, V(*c)))] for c in cursors]
    return got


def test_the_dimension_frames_answer_as_the_python_one_did():
    from core import dimension
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps([answer(dimension, c) for c in cases()]))
    assert len(got) == len(want)
    wrong = [f"case {i} {k}" for i, (g, w) in enumerate(zip(got, want))
             for k in w if g[k] != w[k]]
    assert not wrong, f"{len(wrong)} answers differ: {wrong[:20]}"


def test_the_corpus_asks_for_every_kind():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    kinds = {c[1][1] for w in want for c in w["cursor"]}
    assert kinds == {None, "x", "y", "z"}


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from core import dimension
    data = [answer(dimension, c) for c in cases()]
    GOLDEN.parent.mkdir(exist_ok=True)
    GOLDEN.write_bytes(gzip.compress(json.dumps(data).encode(), mtime=0))
    print(f"wrote {GOLDEN} ({len(data)} cases)")
