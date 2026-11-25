# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The camera's fitting and first-person moves (``fit_box``, ``fit_to``,
``place``, ``look_from``, ``turn``, ``snap_iso``) must keep answering as
``core/camera.py``'s Python did at ``74b54c81`` (before any of it moved), bit for bit, when they move
onto the Rust ``Cam``.

``tests/fixtures/camera_fit_golden.json.gz`` holds the resulting poses
(target, distance, yaw, pitch); regenerate only on purpose, against the
reference implementation: ``python tests/test_camera_fit_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import math
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "camera_fit_golden.json.gz"


def runs():
    """``(name, camera, action)`` seeded."""
    from built_core import Vec3 as V
    from core.camera import OrbitCamera
    rng = random.Random(20261124)
    out = []
    for k in range(120):
        c = OrbitCamera()
        c.set_aspect(rng.randint(300, 2000), rng.randint(300, 1200))
        c.yaw = rng.uniform(-math.pi, math.pi)
        c.pitch = rng.choice((rng.uniform(-1.5, 1.5), math.radians(90.0), math.radians(-90.0)))
        c.distance = 10 ** rng.uniform(-1, 3)
        c.fov_deg = rng.uniform(15, 90)
        c.perspective = rng.random() < 0.6
        c.two_point = rng.random() < 0.15
        if rng.random() < 0.2:
            c.roll_deg = rng.uniform(-90, 90)
        base = (4.8e5, 4.6e6, 20.0) if k % 6 == 0 else (0.0, 0.0, 0.0)
        lo = V(*(b + rng.uniform(-40, 40) for b in base))
        hi = lo + V(rng.uniform(0, 30), rng.uniform(0, 30), rng.uniform(0, 10))
        kind = k % 6
        if kind == 0:
            act = ("fit_box", (lo, hi))
        elif kind == 1:
            free = (rng.uniform(-1, -0.2), rng.uniform(-1, -0.2), rng.uniform(0.2, 1), rng.uniform(0.2, 1))
            act = ("fit_box", (lo, hi, rng.uniform(1.0, 1.5), free))
        elif kind == 2:
            act = ("fit_to", (lo, hi, rng.uniform(1.0, 1.6)))
        elif kind == 3:
            act = ("place", (lo, hi if rng.random() < 0.9 else lo))
        elif kind == 4:
            act = ("turn", (rng.uniform(-90, 90), rng.uniform(-60, 60)))
        else:
            act = ("snap_iso", ())
        out.append((c, act))
    return out


def answer(c, act):
    name, args = act
    got = getattr(c, name)(*args)
    t = c.target
    return {"target": [t.x(), t.y(), t.z()], "distance": c.distance, "yaw": c.yaw,
            "pitch": c.pitch, "returned": got if isinstance(got, bool) else None}


def test_fitting_and_moves_answer_as_the_python_ones_did():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    cases = runs()
    assert len(cases) == len(want)
    wrong = [f"run {i} ({act[0]}) {k}" for i, ((c, act), w) in enumerate(zip(cases, want))
             for k, v in answer(c, act).items() if v != w[k]]
    assert not wrong, f"{len(wrong)} differ: {wrong[:20]}"


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    rows = [answer(c, a) for c, a in runs()]
    GOLDEN.write_bytes(gzip.compress(json.dumps(rows, separators=(",", ":")).encode(),
                                     mtime=0))
    print(len(rows), "runs ->", GOLDEN, GOLDEN.stat().st_size)
