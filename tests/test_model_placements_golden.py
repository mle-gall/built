# SPDX-License-Identifier: GPL-3.0-or-later
"""``core.group.model_placements`` must answer as the Python generator at
``a1240f90`` did, bit for bit: seeded trees (rotations by odd angles,
scales, mirrors, survey-coordinate offsets, untransformed groups between
transformed ones, matrices read back from a Built file's exact values), a
base matrix or none, a filter or none. Each yield is recorded as the uid,
the matrix's float bits (or None) and whether it is the very array the
placement above yielded (an untransformed placement hands its parent's
on). NumPy's 4 x 4 product on this Mac is fma(a2, b2, a0 b0) +
fma(a3, b3, a1 b1) per element (measured on 48,000 elements, all equal);
the port computes it that way. ``tests/fixtures/model_placements_golden.json``
holds the answers; regenerate only on purpose:
``python tests/test_model_placements_golden.py --write``."""
from __future__ import annotations

import json
import random
import struct
import sys
from pathlib import Path

import pytest

GOLDEN = Path(__file__).parent / "fixtures" / "model_placements_golden.json"
CASES = 60


def _walk():
    import built_core
    native = getattr(built_core, "group_model_placements", None)
    if native is not None:
        return native
    from core.group import model_placements
    return model_placements


def tree(seed):
    from built_core import Matrix4
    from core.group import Group
    from core.mesh import Mesh
    rng = random.Random(seed)
    n = [0]

    def make(depth):
        n[0] += 1
        g = Group(Mesh(), name=f"g{n[0]}", uid=f"u{seed}-{n[0]}")
        r = rng.random()
        if r < 0.75:
            m = Matrix4()
            m.translate(rng.uniform(-1e5, 1e5) if rng.random() < 0.3 else rng.uniform(-3, 3),
                        rng.uniform(-3, 3), rng.uniform(-3, 3))
            m.rotate(rng.uniform(0, 360), rng.random(), rng.random(), rng.random() + 0.1)
            if rng.random() < 0.3:
                m.scale(rng.choice([-1.0, 1.0, 0.3, 2.5]), rng.uniform(0.5, 2), 1.0)
            g.xform = m
        if depth < 4 and rng.random() < 0.7:
            g.adopt([make(depth + 1) for _ in range(rng.randint(1, 3))])
        return g
    return make(0), rng


def bits(m):
    import numpy as np
    return None if m is None else [struct.unpack("<q", struct.pack("<d", float(v)))[0]
                                   for v in np.asarray(m).ravel()]


def answers(seed):
    import numpy as np
    walk = _walk()
    root, rng = tree(seed)
    base = None
    if rng.random() < 0.4:
        base = np.asarray(rng.choice([[1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 5.5, -2.25, 1e5, 1],
                                      [0, 1, 0, 0, -1, 0, 0, 0, 0, 0, 1, 0, 0.1, 0.2, 0.3, 1]]),
                          dtype=float).reshape(4, 4)
    hide = {rng.choice("0123456789") for _ in range(2)}
    visible = (lambda g: g.uid[-1] not in hide) if rng.random() < 0.5 else None
    out, last = [], {}
    for g, m in walk(root, base, visible):
        above = getattr(g, "_parent", None)
        same = m is not None and m is last.get(id(above))
        last[id(g)] = m
        out.append([g.uid, bits(m), same])
    return out


@pytest.mark.parametrize("seed", range(CASES))
def test_model_placements_answer_as_the_python_did(seed):
    assert answers(seed) == json.loads(GOLDEN.read_text())[seed]


if __name__ == "__main__":
    if "--write" in sys.argv:
        GOLDEN.write_text(json.dumps([answers(seed) for seed in range(CASES)]))
        print("wrote", GOLDEN)
