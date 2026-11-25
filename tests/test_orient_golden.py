# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""``built_core.orient.orient_outward`` (the native pass, ``orient.rs``) must
decide as the Python oracle of ``core/orient.py`` did at ``0fd95100``, so
the oracle can go: seeded assemblies of boxes (alone, side by side, sharing
a wall that becomes an interior partition, stacked, one inside another),
prisms, an open sheet and a flat drawing, with faces turned over at random
and passes restricted to a component (``only``). Recorded with the scans
off (``BUILT_SCANS=0``: the Python pass); each case keeps the faces
turned, every face's ``interior`` mark, ``is_closed`` and
``signed_volume`` (to 1e-9).

Regenerate only on purpose, against the reference implementation:
``BUILT_SCANS=0 python tests/test_orient_golden.py --write``."""
from __future__ import annotations

import gzip
import json
import math
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "orient_golden.json.gz"


def _box(m, V, x, y, z, sx, sy, sz):
    p = [V(x + a * sx, y + b * sy, z + c * sz) for c in (0, 1) for b in (0, 1) for a in (0, 1)]
    for loop in ((0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4), (2, 6, 7, 3), (0, 4, 6, 2), (1, 3, 7, 5)):
        m.add_face([p[i] for i in loop])


def _prism(m, V, rng, x, y):
    n = rng.randint(3, 9)
    r, h = rng.uniform(0.5, 2), rng.uniform(0.5, 3)
    ring = [(x + r * math.cos(2 * math.pi * i / n), y + r * math.sin(2 * math.pi * i / n)) for i in range(n)]
    m.add_face([V(a, b, 0) for a, b in reversed(ring)])
    m.add_face([V(a, b, h) for a, b in ring])
    for i in range(n):
        (a0, b0), (a1, b1) = ring[i], ring[(i + 1) % n]
        m.add_face([V(a0, b0, 0), V(a1, b1, 0), V(a1, b1, h), V(a0, b0, h)])


def cases():
    from built_core import Vec3 as V
    from built_core.mesh import Mesh, turn_face_over
    rng = random.Random(20261125)
    for k in range(70):
        m = Mesh()
        kind = k % 7
        if kind == 0:
            _box(m, V, 0, 0, 0, rng.uniform(1, 3), rng.uniform(1, 3), rng.uniform(1, 3))
        elif kind == 1:      # side by side, sharing a wall
            _box(m, V, 0, 0, 0, 2, 2, 2)
            _box(m, V, 2, 0, 0, 2, 2, 2)
        elif kind == 2:      # apart
            _box(m, V, 0, 0, 0, 1, 1, 1)
            _box(m, V, 3, 0, 0, 1, 2, 1)
        elif kind == 3:      # stacked
            _box(m, V, 0, 0, 0, 2, 2, 1)
            _box(m, V, 0, 0, 1, 2, 2, 1)
        elif kind == 4:      # one inside another
            _box(m, V, 0, 0, 0, 4, 4, 4)
            _box(m, V, 1, 1, 1, 1, 1, 1)
        elif kind == 5:
            _prism(m, V, rng, 0, 0)
            if rng.random() < 0.5:
                _prism(m, V, rng, 5, 0)
        else:                # open sheet or flat drawing
            m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
            if rng.random() < 0.5:
                m.add_face([V(1, 0, 0), V(2, 0, 0), V(2, 1, 0.5), V(1, 1, 0)])
        faces = list(m.faces)
        for f in faces:
            if rng.random() < 0.35:
                turn_face_over(f)
        only = None
        if len(faces) > 6 and rng.random() < 0.3:
            only = [faces[0]]
        yield m, only, rng.randrange(2 ** 32)


def answers():
    from built_core.orient import is_closed, orient_outward, signed_volume
    out = []
    for m, only, seed in cases():
        faces = list(m.faces)
        index = {f.key: i for i, f in enumerate(faces)}
        turned = orient_outward(m, seed=seed, only=only)
        out.append({"turned": sorted(index[f.key] for f in turned),
                    "interior": [bool(f.interior) for f in m.faces],
                    "closed": bool(is_closed(m)), "volume": float(signed_volume(m))})
    return out


def test_the_native_pass_decides_as_the_python_oracle_did():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = answers()
    assert len(got) == len(want)
    for i, (g, w) in enumerate(zip(got, want)):
        assert g["turned"] == w["turned"], (i, g, w)
        assert g["interior"] == w["interior"], i
        assert g["closed"] == w["closed"], i
        assert abs(g["volume"] - w["volume"]) <= 1e-9 * max(1.0, abs(w["volume"])), i


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    import os
    assert os.environ.get("BUILT_SCANS") == "0", "record with the Python pass"
    GOLDEN.parent.mkdir(exist_ok=True)
    data = json.dumps(answers(), separators=(",", ":")).encode()
    GOLDEN.write_bytes(gzip.compress(data, mtime=0))
    d = json.loads(data)
    print(GOLDEN, len(d), sum(len(x["turned"]) for x in d), sum(sum(x["interior"]) for x in d))
