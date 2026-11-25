# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""``built_core.texture.continuous_maps`` (a texture walked across soft edges)
moved to Rust (``model/texture_maths.rs``) and must answer as the Python
did at ``fff1954a``: seeded prisms and rounded corners of 3 to 24 facets,
some edges hard, holes, folds sharper than the hinge allows, degenerate
facets, seeds inside and outside the set, textures with and without their
own ``uvw`` / ``rot``. The maps are compared bit for bit.

Regenerate only on purpose, against the reference implementation:
``python tests/test_continuous_maps_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import math
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "continuous_maps_golden.json.gz"


def _prism(rng):
    from built_core import Vec3 as V
    from built_core.mesh import Mesh
    m = Mesh()
    n = rng.randint(3, 24)
    r, h = rng.uniform(0.2, 5), rng.uniform(0.5, 4)
    sweep = rng.choice((2 * math.pi, math.pi / 2, math.pi * 1.9))
    closed = sweep == 2 * math.pi
    steps = n if closed else n + 1
    ring = []
    for i in range(steps):
        a = sweep * i / n
        rr = r * (1 + (rng.uniform(-0.3, 0.3) if rng.random() < 0.2 else 0))
        ring.append((rr * math.cos(a), rr * math.sin(a)))
    faces = []
    for i in range(n if closed else n):
        (x0, y0), (x1, y1) = ring[i], ring[(i + 1) % steps]
        f = m.add_face([V(x0, y0, 0), V(x1, y1, 0), V(x1, y1, h), V(x0, y0, h)])
        if f is not None:
            faces.append(f)
    if rng.random() < 0.3 and faces:
        # A fold back onto the first facet: no hinge.
        (x0, y0), (x1, y1) = ring[0], ring[1]
        f = m.add_face([V(x0, y0, 0), V(x0, y0, h), V(x1 * 0.999 + x0 * 0.001, y1 * 0.999 + y0 * 0.001, h * 0.5)])
        if f is not None:
            faces.append(f)
    for e in m.edges:
        vertical = abs(e.v0.position.z() - e.v1.position.z()) > 1e-9
        if vertical and rng.random() < 0.85:
            e.soft = True
    return m, faces


def _tex(rng):
    tex = {"path": "/x/brick.png", "sw": rng.choice((1.0, 0.5, 2.25)), "sh": rng.choice((1.0, 0.3))}
    if rng.random() < 0.4:
        tex["rot"] = rng.uniform(-180, 180)
    if rng.random() < 0.3:
        tex["uvw"] = [rng.uniform(-1, 1) for _ in range(8)]
    if rng.random() < 0.3:
        tex["tint"] = [0.1, 0.2, 0.3]
    return tex


def answers():
    from built_core import texture_continuous_maps as continuous_maps
    rng = random.Random(20261125)
    out = []
    for k in range(150):
        m, faces = _prism(rng)
        index = {f.key: i for i, f in enumerate(m.faces)}
        chosen = [f for f in faces if rng.random() < 0.9]
        seed = rng.choice(faces) if faces and rng.random() < 0.95 else None
        tex = _tex(rng)
        got = continuous_maps(m, chosen, seed, tex)
        out.append(sorted([index[key], {k2: (v if k2 != "uvw" else [float(x) for x in v])
                                        for k2, v in sorted(t.items())}]
                          for key, t in got.items()))
    return out


def test_continuous_maps_answer_as_the_python_did():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(answers()))
    assert len(got) == len(want)
    bad = [i for i, (g, w) in enumerate(zip(got, want)) if g != w]
    assert not bad, bad[:10]


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    GOLDEN.parent.mkdir(exist_ok=True)
    data = json.dumps(answers(), separators=(",", ":")).encode()
    GOLDEN.write_bytes(gzip.compress(data, mtime=0))
    print(GOLDEN, len(data), sum(len(x) for x in json.loads(data)))
