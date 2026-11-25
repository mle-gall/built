# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The camera's pose and projection moved to Rust
(``rust/built_core/src/sheets/camera.rs``'s ``Cam``, the one the sheets
already used) and must answer bit for bit as ``core/camera.py``'s Python
did at ``74b54c81``.

``tests/fixtures/camera_golden.json.gz`` holds those answers for every
camera of :func:`cameras`: eye, forward, up, view and projection matrices,
the picking projection, near / far / parallel depth and the closest
approach, with and without a depth range fitted to boxes. Regenerate only
on purpose, against the reference implementation:
``python tests/test_camera_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import math
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "camera_golden.json.gz"


def cameras():
    """Seeded cameras: standard views, random poses, far from the origin,
    parallel and perspective, two-point, shifted, rolled, clipped, with a
    depth reach, and fitted to boxes."""
    from built_core import Vec3 as V
    from built_core.camera import OrbitCamera
    rng = random.Random(20261124)
    out = []
    for view in ("top", "bottom", "front", "back", "right", "left", "iso"):
        for persp in (True, False):
            c = OrbitCamera()
            c.set_view(view)
            c.perspective = persp
            c.set_aspect(1280, 800)
            out.append((c, None))
    for k in range(160):
        c = OrbitCamera()
        c.target = V(rng.uniform(-50, 50), rng.uniform(-50, 50), rng.uniform(-5, 20))
        if k % 9 == 0:
            c.target = V(rng.uniform(4e5, 5e5), rng.uniform(4.6e6, 4.7e6), 30.0)
        c.distance = 10 ** rng.uniform(-2.3, 3.8)
        c.yaw = rng.uniform(-math.pi, math.pi)
        c.pitch = rng.choice((rng.uniform(-1.55, 1.55), math.radians(90.0),
                              math.radians(-90.0), math.radians(89.0)))
        c.fov_deg = rng.uniform(10, 100)
        c.aspect = rng.uniform(0.5, 2.5)
        c.perspective = rng.random() < 0.6
        c.two_point = rng.random() < 0.25
        if rng.random() < 0.3:
            c.shift_x, c.shift_y = rng.uniform(-0.5, 0.5), rng.uniform(-0.5, 0.5)
        if rng.random() < 0.3:
            c.roll_deg = rng.uniform(-180, 180)
        if rng.random() < 0.2:
            c.clip_near = rng.uniform(0.01, 5)
        if rng.random() < 0.2:
            c.clip_far = rng.uniform(1, 500)
        if rng.random() < 0.4:
            c.depth_reach = (V(rng.uniform(-30, 30), rng.uniform(-30, 30), 0.0),
                             rng.uniform(0.1, 300))
        if rng.random() < 0.1:
            c.up = V(rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-1, 1))
        boxes = None
        if rng.random() < 0.4:
            n = rng.randint(1, 6)
            lo = [[c.target.x() + rng.uniform(-30, 30), c.target.y() + rng.uniform(-30, 30),
                   c.target.z() + rng.uniform(-10, 10)] for _ in range(n)]
            hi = [[a + rng.uniform(0.1, 20) for a in p] for p in lo]
            boxes = (lo, hi)
        out.append((c, boxes))
    return out


def answer(c, boxes):
    if boxes is not None:
        c.fit_depth_range(*boxes)

    def tup(v):
        return [v.x(), v.y(), v.z()]
    return {"eye": tup(c.eye()), "forward": tup(c.forward()), "up": tup(c.up_vector()),
            "view": list(c.view_matrix().data()),
            "projection": list(c.projection_matrix().data()),
            "picking": list(c.picking_projection_matrix().data()),
            "near": c.near_plane(), "far": c.far_plane(),
            "parallel_depth": c.parallel_depth(), "min_distance": c.min_distance(),
            "fitted": c._depth_range is not None}


def _jsonable(x):
    if isinstance(x, float) and not math.isfinite(x):
        return repr(x)
    if isinstance(x, list):
        return [_jsonable(v) for v in x]
    if isinstance(x, dict):
        return {k: _jsonable(v) for k, v in x.items()}
    return x


def test_the_camera_answers_as_the_python_one_did():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    cases = cameras()
    assert len(cases) == len(want)
    wrong = []
    for i, ((c, boxes), expected) in enumerate(zip(cases, want)):
        got = _jsonable(answer(c, boxes))
        for key in expected:
            if got[key] != expected[key]:
                wrong.append(f"camera {i} {key}: {got[key]!r} != {expected[key]!r}")
    assert not wrong, f"{len(wrong)} answers differ:\n" + "\n".join(wrong[:20])


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    rows = [_jsonable(answer(c, b)) for c, b in cameras()]
    GOLDEN.write_bytes(gzip.compress(json.dumps(rows, separators=(",", ":")).encode(),
                                     mtime=0))
    print(len(rows), "cameras ->", GOLDEN)


def test_the_cached_fields_follow_every_change():
    """The Rust copy of the fields (``_rs_cam``) is dropped on every write
    and the vectors checked again on use: a target changed in place, a
    field set, a copy, a pickle all read as the camera they are."""
    import copy
    import pickle
    from built_core import Vec3 as V
    from built_core.camera import OrbitCamera
    c = OrbitCamera()
    first = list(c.view_matrix().data())
    assert "_rs_cam" in c.__dict__
    assert list(copy.deepcopy(c).view_matrix().data()) == first
    assert list(pickle.loads(pickle.dumps(c)).view_matrix().data()) == first
    c.target.setX(5.0)                       # in place: no __setattr__
    moved = list(c.view_matrix().data())
    assert moved != first
    c.distance = 3.0
    assert "_rs_cam" not in c.__dict__
    fresh = OrbitCamera()
    fresh.target = V(5.0, 0.0, 0.0)
    fresh.distance = 3.0
    assert list(c.view_matrix().data()) == list(fresh.view_matrix().data())
    assert list(c.projection_matrix().data()) == list(fresh.projection_matrix().data())
