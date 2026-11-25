# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The rest of the hidden-line maths moved to Rust
(``rust/ingetrazo_core/src/model/hlr_view.rs``) and must answer as
``core/hlr.py``'s NumPy did at ``9cf53080``: the camera basis, how deep a
cut solid runs (``solid_depth``), the cap's depth plane and ring edges, the
clips against the view pyramid, and the whole perspective drawing.

Bit for bit, but where the NumPy went through BLAS or LAPACK and its order
is not one we can name: ``solid_depth`` (``einsum``) and the cap's
``linalg.solve`` are held to 1e-9 relative; the perspective's clip-space
products (``(N, 3) @ (3, 4)``, a fused chain at most sizes) are run as that
chain, and its segments held to 1e-9 of the drawing's size.

Regenerate only on purpose, against the reference implementation:
``python tests/test_hlr_rest_parity.py --write``."""
from __future__ import annotations

import ingetrazo_core
import gzip
import json
import math
import random
import sys
from pathlib import Path

import numpy as np

GOLDEN = Path(__file__).parent / "fixtures" / "hlr_rest_golden.json.gz"


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


class _Plane:
    def __init__(self, point, normal):
        from ingetrazo_core import Vec3 as V
        self.point, self.normal = V(*point), V(*normal).normalized()

    def side(self, p):
        from ingetrazo_core import Vec3 as V
        return V.dotProduct(self.normal, p - self.point)


class _Scene:
    def __init__(self, plane=None):
        self.plane = plane
        self.show_section_cuts = True

    def active_section(self):
        return self.plane


def _camera(rng, perspective=True):
    from core.camera import OrbitCamera
    from core.camera_point import CameraPoint
    c = OrbitCamera()
    c.target = CameraPoint((rng.uniform(-3, 3), rng.uniform(-3, 3), rng.uniform(0, 2)))
    c.yaw = rng.uniform(-math.pi, math.pi)
    c.pitch = rng.uniform(-1.2, 1.2)
    c.distance = rng.uniform(4, 40)
    c.fov_deg = rng.choice((35.0, 45.0, 60.0))
    c.aspect = rng.choice((1.0, 1.5, 0.75))
    c.perspective = perspective
    c.two_point = rng.random() < 0.2
    return c


def scenes():
    rng = random.Random(20261124)
    out = []
    for k in range(30):
        tris, hard = [], []
        for _ in range(rng.randint(1, 4)):
            t, h = _box(rng.uniform(-6, 6), rng.uniform(-6, 6), rng.uniform(-1, 1),
                        rng.uniform(0.2, 3), rng.uniform(0.2, 3), rng.uniform(0.5, 4))
            tris += t
            hard += h
        soft = np.array([[h[0], h[1]] for h in hard[: len(hard) // 4]], dtype=np.float64).reshape(-1, 2, 3)
        soft_n = np.array([[(0.0, 0.0, 1.0), (1.0, 0.0, 0.0) if i % 3 else (np.nan,) * 3]
                           for i in range(len(soft))], dtype=np.float64).reshape(-1, 2, 3)
        plane = None
        if k % 3 == 1:
            plane = _Plane((rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(0, 2)),
                           (rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-1, 1)))
        out.append((np.array(tris, dtype=np.float64), np.array(hard, dtype=np.float64),
                    soft, soft_n, plane, _camera(rng)))
    return out


def _r(x):
    if isinstance(x, np.ndarray):
        return ["A", list(x.shape), [repr(v) if not math.isfinite(v) else v
                                     for v in x.astype(np.float64).ravel().tolist()]]
    if isinstance(x, (list, tuple)):
        return [_r(v) for v in x]
    if isinstance(x, (np.floating, float)):
        return float(x) if math.isfinite(x) else repr(float(x))
    return x


def answers(hlr):
    rng = random.Random(5)
    out = {"basis": [], "depth": [], "cap": [], "ring": [], "clip_t": [], "clip_s": [],
           "persp": []}
    for tris, hard, soft, soft_n, plane, cam in scenes():
        out["basis"].append(_r(list(ingetrazo_core.hlr_camera_basis(cam))))
        for persp in (False, True):
            cam.perspective = persp
            out["basis"].append(_r(list(ingetrazo_core.hlr_camera_basis(cam))))
        cam.perspective = True
        e, r, u, f = ingetrazo_core.hlr_camera_basis(cam)
        for _ in range(3):
            pl = _Plane((rng.uniform(-2, 2), rng.uniform(-2, 2), rng.uniform(0, 2)),
                        (rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-1, 1)))
            out["cap"].append(_r(ingetrazo_core.hlr_cap_depth_affine(pl, e, r, u, f)))
        out["cap"].append(_r(ingetrazo_core.hlr_cap_depth_affine(_Plane((0, 0, 0), tuple(f)), e, r, u, f)))
        rings = [np.array([(x, y, 0.5) for x, y in ((0, 0), (1, 0), (1, 1), (0, 1))], float)]
        b = tris[:12].reshape(-1, 3, 3)
        lo = b.reshape(-1, 3).min(axis=0)
        hi = b.reshape(-1, 3).max(axis=0)
        mid = (lo + hi) / 2
        ring = np.array([mid + (dx, dy, 0) for dx, dy in ((-.05, -.05), (.05, -.05), (.05, .05), (-.05, .05))])
        out["depth"].append(_r(ingetrazo_core.hlr_solid_depth(ring, b, (0.0, 0.0, 1.0))))
        out["depth"].append(_r(ingetrazo_core.hlr_solid_depth(ring, b, (0.3, -0.2, 1.0))))
        out["depth"].append(_r(ingetrazo_core.hlr_solid_depth(ring, tris, (1.0, 0.0, 0.0))))
        out["depth"].append(_r(ingetrazo_core.hlr_solid_depth(ring + (100, 0, 0), b, (0.0, 0.0, 1.0))))
        out["depth"].append(_r(ingetrazo_core.hlr_solid_depth(ring[:2], b, (0.0, 0.0, 1.0))))
        rings.append(ring)
        out["ring"].append(_r(list(ingetrazo_core.hlr_cap_ring_edges(rings))))
        # Clips in clip space, straight from the view.
        m = cam.projection_matrix() * cam.view_matrix()
        M = np.array(m.data(), dtype=np.float64).reshape(4, 4, order="F")
        C = np.concatenate([tris.reshape(-1, 3), np.ones((len(tris) * 3, 1))], axis=1) @ M.T
        out["clip_t"].append(_r(ingetrazo_core.hlr_clip_triangles(C.reshape(-1, 3, 4))))
        E = np.concatenate([hard.reshape(-1, 3), np.ones((len(hard) * 2, 1))], axis=1) @ M.T
        E = E.reshape(-1, 2, 4)
        out["clip_s"].append(_r(list(ingetrazo_core.hlr_clip_segments(E[:, 0], E[:, 1]))))
        out["persp"].append(_r(hlr.hlr_perspective(_Scene(plane), cam,
                                                   geometry=(tris, hard, soft, soft_n))))
    out["ring"].append(_r(list(ingetrazo_core.hlr_cap_ring_edges([]))))
    out["clip_t"].append(_r(ingetrazo_core.hlr_clip_triangles(np.zeros((0, 3, 4)))))
    out["depth"].append(_r(ingetrazo_core.hlr_solid_depth(np.zeros((4, 3)), np.zeros((0, 3, 3)), (0, 0, 1))))
    return out


def _close(g, w, rel, scale=1.0):
    if isinstance(w, list) and isinstance(g, list):
        if len(w) == 3 and w and w[0] == "A":
            if g[0] != "A" or g[1] != w[1]:
                return False
            vals = [v for v in w[2] if isinstance(v, float)]
            s = max([1.0] + [abs(v) for v in vals])
            return _close(g[2], w[2], rel, s)
        return len(g) == len(w) and all(_close(a, b, rel, scale) for a, b in zip(g, w))
    if isinstance(w, float) and isinstance(g, (int, float)):
        return abs(g - w) <= rel * max(scale, abs(w))
    return g == w


TOLERANCE = {"depth": 1e-9, "cap": 1e-9, "persp": 1e-9}


def test_the_hidden_line_rest_answers_as_the_numpy_one_did():
    from core import hlr
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(answers(hlr)))
    wrong = []
    for key, w in want.items():
        g = got[key]
        rel = TOLERANCE.get(key)
        bad = [i for i, (a, b) in enumerate(zip(g, w))
               if (a != b if rel is None else not _close(a, b, rel))]
        if len(g) != len(w) or bad:
            wrong.append(f"{key}: {len(bad)} of {len(w)}, first {bad[:5]}")
    assert not wrong, wrong


def test_the_corpus_measures_and_draws():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    assert sum(1 for d in want["depth"] if isinstance(d, float)) > 40
    assert sum(1 for p in want["persp"] if p[1][0] > 0) > 25


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from core import hlr
    data = answers(hlr)
    GOLDEN.parent.mkdir(exist_ok=True)
    GOLDEN.write_bytes(gzip.compress(json.dumps(data).encode(), mtime=0))
    print(f"wrote {GOLDEN}")
