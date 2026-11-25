# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The texture maths moved to Rust (``rust/ingetrazo_core/src/model/texture_maths.rs``)
and must answer as ``core/texture.py``'s Python did at ``9cf53080``: the
.skp projection basis and axes, planar and fitted UVs, the map a face renders
with, the placement a fitted map shows, the hinge walk's turned map, the
reference triad, and the HLS colour maths with the colourize pass.

``tests/fixtures/texture_maths_golden.json.gz`` holds those answers on seeded
normals (vertical ones carrying float noise near the .skp tolerance included),
polygons and UVs, maps and colours. Everything is compared bit for bit but
``fit_uv_affine`` on more than 13 points, where the Python searched the best
edge pair with a NumPy Gram matmul (BLAS: its order changes with the size):
there a tied pair may be chosen in another order, and the map, the same plane
map, is held to 1e-9 relative. Regenerate only on purpose, against the
reference implementation: ``python tests/test_texture_maths_parity.py --write``."""
from __future__ import annotations

import ingetrazo_core
import gzip
import json
import math
import random
import sys
from pathlib import Path

import numpy as np

GOLDEN = Path(__file__).parent / "fixtures" / "texture_maths_golden.json.gz"


def _normals(rng):
    out = [(0, 0, 1), (0, 0, -1), (1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0),
           (0.0, 0.0, 0.0), (1e-10, 0, 1), (0, 1e-10, 1), (9.9e-4, 0, 1), (1.0001e-3, 0, 1),
           (0, -1.0001e-3, -1), (6e-4, -3e-4, 1)]
    for _ in range(80):
        out.append((rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-1, 1)))
    for _ in range(20):
        out.append((rng.uniform(-2e-3, 2e-3), rng.uniform(-2e-3, 2e-3), rng.choice((1.0, -1.0))))
    return [tuple(float(c) for c in n) for n in out]


def _polygon(rng, n, regular=False):
    from ingetrazo_core import Vec3 as V
    nx, ny, nz = rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-1, 1)
    from ingetrazo_core.triangulate import plane_axes
    nn = V(nx, ny, nz).normalized()
    if nn.length() == 0:
        nn = V(0, 0, 1)
    e1, e2 = plane_axes(nn)
    c = V(rng.uniform(-50, 50), rng.uniform(-50, 50), rng.uniform(-5, 5))
    r = rng.uniform(0.05, 5)
    pts = []
    for k in range(n):
        a = 2 * math.pi * k / n if regular else rng.uniform(0, 2 * math.pi)
        rr = r if regular else r * rng.uniform(0.3, 1.0)
        pts.append(c + e1 * (rr * math.cos(a)) + e2 * (rr * math.sin(a)))
    if not regular:
        pts.sort(key=lambda p: math.atan2(QV.dotProduct(p - c, e2), QV.dotProduct(p - c, e1)))
    return [p.toTuple() for p in pts], (nn.x(), nn.y(), nn.z())


from ingetrazo_core import Vec3 as QV  # noqa: E402


def cases():
    rng = random.Random(20261124)
    normals = _normals(rng)
    polys = []
    for k in range(70):
        n = rng.choice((3, 4, 5, 8, 12, 13, 14, 20, 40)) if k < 60 else rng.choice((24, 32))
        pts, nrm = _polygon(rng, n, regular=k >= 60 or rng.random() < 0.2)
        a, b, c = rng.uniform(-2, 2), rng.uniform(-2, 2), rng.uniform(-2, 2)
        uvs = [(a * x + b * y + rng.uniform(-1, 1) * 0 + c, b * x - a * z + 0.5)
               for x, y, z in pts]
        polys.append((pts, uvs, nrm))
    polys.append(([(0, 0, 0), (1, 0, 0), (2, 0, 0)], [(0, 0)] * 3, (0, 0, 1)))  # collinear
    polys.append(([(0, 0, 0), (1, 0, 0)], [(0, 0)] * 2, (0, 0, 1)))             # too few
    texes = [{"sw": rng.choice((0.5, 1.0, 2.0, 0.0)), "sh": rng.choice((0.25, 1.0, 3.0)),
              "rot": rng.choice((0.0, 15.0, 90.0, -33.3, None))} for _ in range(30)]
    colours = [(rng.random(), rng.random(), rng.random()) for _ in range(200)]
    colours += [(0, 0, 0), (1, 1, 1), (0.5, 0.5, 0.5), (1, 0, 0), (0, 1, 0), (0, 0, 1),
                (1, 1, 0), (0.2, 0.2, 0.9), (0.9, 0.1, 0.1)]
    return normals, polys, texes, colours


def _r(x):
    if isinstance(x, QV):
        return ["V", x.x(), x.y(), x.z()]
    if isinstance(x, np.ndarray):
        return ["A", list(x.shape), x.astype(np.float64).ravel().tolist()]
    if isinstance(x, (list, tuple)):
        return [_r(v) for v in x]
    if isinstance(x, dict):
        return {k: _r(v) for k, v in sorted(x.items())}
    if isinstance(x, (np.floating,)):
        return float(x)
    if isinstance(x, float) and not math.isfinite(x):
        return repr(x)
    return x


def answers(texture):
    normals, polys, texes, colours = cases()
    out = {}
    out["basis"] = [_r(texture.projection_basis(n)) for n in normals]
    out["axes"] = [_r(ingetrazo_core.texture_projection_axes(QV(*n), rot))
                   for n in normals for rot in (0.0, 30.0, -122.5)]
    pts0 = [QV(1.5, -2.25, 3.0), QV(0, 0, 0), QV(-10.1, 4.4, 0.7)]
    out["planar"] = [_r(ingetrazo_core.texture_planar_uv(QV(*n), pts0, sw, sh, rot))
                     for n in normals[:40] for sw, sh, rot in ((1.0, 1.0, 0.0), (0.3, 2.0, 45.0),
                                                               (0.0, 1e-12, -10.0))]
    fits, fits_big = [], []
    for pts, uvs, _n in polys:
        f = ingetrazo_core.texture_fit_uv_affine(pts, uvs)
        (fits_big if len(pts) > 13 else fits).append(_r(f))
        fits.append(_r(ingetrazo_core.texture_fit_uv_affine([QV(*p) for p in pts[:5]], uvs[:5])))
    out["fit"] = fits
    out["fit_big"] = fits_big
    out["affine"] = [_r(ingetrazo_core.texture_affine_uv(ingetrazo_core.texture_fit_uv_affine(p, u) or [1, 2, 3, 4, 5, 6, 7, 8],
                                          p + [QV(*p[0])]))
                     for p, u, _n in polys]
    maps = []
    for i, tex in enumerate(texes):
        n = normals[i % len(normals)]
        t = {k: v for k, v in tex.items() if v is not None}
        maps.append(_r(texture.face_uv_axes(t, QV(*n))))
        pts, uvs, nrm = polys[i]
        uvw = ingetrazo_core.texture_fit_uv_affine(pts, uvs)
        if uvw:
            maps.append(_r(texture.face_uv_axes({"uvw": uvw}, QV(*nrm))))
            maps.append(_r(ingetrazo_core.texture_placement_of({"uvw": uvw}, QV(*nrm))))
            maps.append(_r(ingetrazo_core.texture_placement_of({"uvw": uvw}, QV(*n))))
            maps.append(_r(ingetrazo_core.texture_flattened({"uvw": uvw, "path": "x.png", "rot": 3},
                                                     QV(*nrm))))
    maps.append(_r(ingetrazo_core.texture_placement_of({"uvw": [0] * 8}, QV(0, 0, 1))))
    maps.append(_r(ingetrazo_core.texture_placement_of({"uvw": [1, 2]}, QV(0, 0, 1))))
    maps.append(_r(ingetrazo_core.texture_placement_of({"uvw": [1, 0, 0, 0, 0, 1, 0, 0]}, QV(0, 0, 0))))
    out["maps"] = maps
    rot = []
    rng = random.Random(7)
    for _ in range(40):
        g = [QV(rng.uniform(-3, 3), rng.uniform(-3, 3), rng.uniform(-3, 3)) for _ in range(2)]
        d = QV(rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-1, 1)).normalized()
        t = rng.uniform(-3, 3)
        rot.append(_r(__import__("ingetrazo_core").texture_rotated_map(g[0], rng.uniform(-2, 2), g[1], rng.uniform(-2, 2),
                                           QV(rng.uniform(-9, 9), rng.uniform(-9, 9), 1.0), d,
                                           math.cos(t), math.sin(t))))
    out["rotated"] = rot
    refs = []
    for pts, _uvs, nrm in polys:
        refs.append(_r(ingetrazo_core.texture_uv_reference_points(pts, None)))
        refs.append(_r(ingetrazo_core.texture_uv_reference_points([QV(*p) for p in pts], QV(*nrm))))
    refs.append(_r(ingetrazo_core.texture_uv_reference_points([(0, 0, 0)] * 4)))
    out["refs"] = refs
    rgb = np.array(colours, dtype=np.float64)
    h, l, s = ingetrazo_core.texture_rgb_to_hls(rgb)
    out["hls"] = _r([h, l, s])
    out["hls_img"] = _r(list(ingetrazo_core.texture_rgb_to_hls(rgb.reshape(-1, 11, 3))))
    out["rgb"] = _r(ingetrazo_core.texture_hls_to_rgb(h, l, s))
    out["rgb_wrap"] = _r(ingetrazo_core.texture_hls_to_rgb(h * 3 - 1.2, l, s))
    return out


def colorize_cases():
    """PNG images to tint, seeded: opaque, with a cutout, a flat colour."""
    from ingetrazo_render import SurveyRaster
    rng = np.random.default_rng(20261124)
    imgs = []
    for w, h, cut in ((17, 9, False), (32, 32, True), (5, 3, False), (64, 40, True)):
        px = rng.integers(0, 256, size=(h, w, 4), dtype=np.uint8)
        px[..., 3] = 255
        if cut:
            px[: h // 3, :, 3] = 0
            px[h // 3: h // 2, :, 3] = 128
        imgs.append(SurveyRaster(w, h, px.tobytes()).material_png(b""))
    flat = np.full((4, 4, 4), 200, np.uint8)
    imgs.append(SurveyRaster(4, 4, flat.tobytes()).material_png(b""))
    return imgs


def colorize_answers(texture):
    from ingetrazo_render import SurveyRaster
    out = []
    for data in colorize_cases():
        for target, mode in (((200, 30, 30), 0), ((10, 200, 90), 1), ((128, 128, 128), 0),
                             ((0, 0, 0), 1), ((255, 255, 255), 0)):
            got = texture.colorize_image(data, target, mode)
            img = SurveyRaster.decode(bytes(got))
            out.append([img.width, img.height, list(img.rgba())])
    out.append(texture.colorize_image(b"not an image", (1, 2, 3)) == b"not an image")
    return out


def _close(g, w, rel):
    if isinstance(w, list) and isinstance(g, list):
        return len(g) == len(w) and all(_close(a, b, rel) for a, b in zip(g, w))
    if isinstance(w, float) and isinstance(g, (int, float)):
        return abs(g - w) <= rel * max(1.0, abs(w))
    return g == w


def test_the_texture_maths_answers_as_the_python_one_did():
    from core import texture
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(answers(texture)))
    wrong = []
    for key, w in want["maths"].items():
        if key == "fit_big":
            if not _close(got[key], w, 1e-9):
                wrong.append(key)
            continue
        if got[key] != w:
            bad = [i for i, (a, b) in enumerate(zip(got[key], w)) if a != b]
            wrong.append(f"{key}: {len(bad)} of {len(w)}, first {bad[:5]}")
    assert not wrong, wrong


def test_colorize_answers_as_the_numpy_one_did():
    from core import texture
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(colorize_answers(texture)))
    assert got == want["colorize"]


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from core import texture
    data = {"maths": answers(texture), "colorize": colorize_answers(texture)}
    GOLDEN.parent.mkdir(exist_ok=True)
    GOLDEN.write_bytes(gzip.compress(json.dumps(data).encode(), mtime=0))
    print(f"wrote {GOLDEN}")
