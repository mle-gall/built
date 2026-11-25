# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""What the retired Python passes of :mod:`built_core.vista_caras` answered.

The faces a view shows were worked out twice: by the Rust engine
(``faces_view``, ``visible_parts``) and by a Python pass of the same rules
(``_face_geometry_py``, ``_visible_parts_py``, with its overlap grid and
batched manifold3d unions). The Python pass is gone;
the tests that held the engine to it compare with its answers here, taken at
``6f3a2576`` on the same scenes (``tests/fixtures/vista_caras_python.json.gz``).

Regenerate only against that reference implementation:
``git checkout 6f3a2576 -- core/vista_caras.py`` and
``python tests/vista_caras_reference.py --write``."""
from __future__ import annotations

import gzip
import json
import sys
from functools import lru_cache
from pathlib import Path

import numpy as np

GOLDEN = Path(__file__).parent / "fixtures" / "vista_caras_python.json.gz"
KERB = Path(__file__).parent / "data" / "plaza_kerb_fan.json"


# ---- Scenes ------------------------------------------------------------------

def _square(x, y, s):
    return np.array([[x, y], [x + s, y], [x + s, y + s], [x, y + s]],
                    dtype=np.float64)


def _scene(rng, n, roof=True):
    """Small tiles at random heights over a floor, under a roof covering
    half of them: the faces a view of a building is made of."""
    from built_core.vista_caras import FaceFill
    faces = []

    def face(ring, z, tilt=(0.0, 0.0)):
        faces.append(FaceFill(rings=[ring], rgb=(0.5, 0.5, 0.5),
                              layer="0", depth=-z,
                              extra={"near": (tilt[0], tilt[1], z)}))
    face(_square(0, 0, 10), 0.0)                              # the floor
    for _ in range(n):
        x, y = rng.uniform(0, 9.5, size=2)
        face(_square(x, y, rng.uniform(0.1, 0.5)),
             float(rng.uniform(0.1, 2.0)),
             tuple(rng.uniform(-0.05, 0.05, size=2)))
    if roof:
        face(_square(0, 0, 5), 3.0)
    return faces


def copies(faces):
    from built_core.vista_caras import FaceFill
    return [FaceFill(**vars(f)) for f in faces]


def vp_same_faces():
    return copies(_scene(np.random.default_rng(33), 400))


def vp_holes():
    from built_core.vista_caras import FaceFill
    faces = copies(_scene(np.random.default_rng(8), 120))
    outer = _square(2, 2, 4)
    hole = _square(3, 3, 2)[::-1].copy()
    faces.append(FaceFill(rings=[outer, hole], rgb=(0.5, 0.5, 0.5),
                          layer="0", depth=-2.5,
                          extra={"near": (0.0, 0.0, 2.5)}))
    faces.append(FaceFill(rings=[_square(0, 0, 10)], rgb=(0.5, 0.5, 0.5),
                          layer="0", depth=-4.0, opacity=0.5,
                          extra={"near": (0.0, 0.0, 4.0)}))
    return faces


def vp_grazing():
    from built_core.vista_caras import FaceFill
    slab = FaceFill(rings=[_square(0, 0, 6), _square(2, 2, 2)[::-1].copy()],
                    rgb=(0, 0, 0), layer="0", depth=0.0,
                    extra={"near": (0.0, 0.0, 0.0)})
    box = FaceFill(rings=[_square(1, 1, 1)], rgb=(0, 0, 0), layer="0",
                   depth=-1.0, extra={"near": (0.0, 0.0, 1.0)})
    return [slab, box]


def vp_coplanar():
    from built_core.vista_caras import FaceFill
    a = FaceFill(rings=[_square(0, 0, 2)], rgb=(0, 0, 0), layer="0",
                 depth=-1.0, extra={"near": (0.0, 0.0, 1.0)})
    b = FaceFill(rings=[_square(1, 1, 2)], rgb=(0, 0, 0), layer="0",
                 depth=-1.0, extra={"near": (0.0, 0.0, 1.0)})
    return [a, b]


def vp_hair():
    from built_core.vista_caras import FaceFill
    faces = copies(_scene(np.random.default_rng(2), 60))
    t = np.linspace(0, 1, 200)
    hair = np.column_stack([3 + t, 5 + 1e-7 * (t * 2e6 % 2)])
    hair = np.vstack([hair, hair[::-1] + [0, 2e-7]])
    faces.append(FaceFill(rings=[hair], rgb=(0, 0, 0), layer="0", depth=-5.0,
                          extra={"near": (0.0, 0.0, 5.0)}))
    return faces


def _face(ring, near, depth):
    from built_core.vista_caras import FaceFill
    return FaceFill(rings=[ring], rgb=(0, 0, 0), layer="0", opacity=1.0,
                    depth=depth, extra={"near": near})


def vp_quarters():
    rng = np.random.default_rng(7)
    faces = [_face(_square(0.0, 0.0, 30.0), (0.0, 0.0, 0.0), 0.0)]  # ground
    for i in range(30):
        for j in range(30):
            s = 0.6 + 0.2 * rng.random()
            faces.append(_face(_square(i + 0.1, j + 0.1, s),
                               (0.0, 0.0, 1.0 + 0.001 * (i + j)), 1.0))
    for k in range(12):
        x = 2.0 + 2.2 * k
        faces.append(_face(_square(x, 5.0 + k, 1.7),
                           (0.3, 0.0, -0.3 * (x + 0.8)), 0.5))
    faces.append(_face(np.array([(3.0, 14.2), (27.0, 14.6), (27.0, 15.4),
                                 (3.0, 15.8)], float), (0.0, 0.0, 2.0), 2.0))
    faces.append(_face(np.array([(14.3, 2.0), (15.7, 2.0), (15.2, 28.0),
                                 (14.8, 28.0)], float), (0.0, 0.0, 2.5), 2.5))
    return faces


def vp_kerb():
    from built_core.vista_caras import FaceFill
    return [FaceFill(rings=[np.array(r, float) for r in f["rings"]],
                     rgb=(0, 0, 0), layer="0", opacity=f["opacity"],
                     depth=f["depth"], extra={"near": tuple(f["near"])})
            for f in json.load(open(KERB))]


VISIBLE = {"same_faces": vp_same_faces, "holes": vp_holes,
           "grazing": vp_grazing, "coplanar": vp_coplanar, "hair": vp_hair,
           "quarters": vp_quarters, "kerb": vp_kerb}


def geometry_cases():
    """``{name: (items, camera, projector, section)}`` of the face-geometry
    comparisons (``tests/test_faces_view_rust.py``'s scenes)."""
    from built_core import Vec3 as V
    from built_core import vista_caras as VC
    from built_core.camera import OrbitCamera
    from built_core.scene import Scene
    from built_core.section import SectionPlane
    from tests.test_faces_view_rust import _camera, _scene as model
    out = {}
    for persp in (False, True):
        for cut in (False, True):
            scene = model()
            cam = _camera(scene, persp)
            proj = VC._Perspective(cam) if persp else VC._Parallel(cam)
            sp = SectionPlane(V(3, 3, 0.7), V(0, 0, 1)) if cut else None
            out[f"{'persp' if persp else 'parallel'}{'_cut' if cut else ''}"] = (
                list(VC._faces_of(scene)), cam, proj, sp)
    front = Scene()
    front.loose_mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    front.version += 1
    cam = OrbitCamera()
    cam.set_aspect(800, 600)
    cam.perspective = True
    cam.set_view("iso")
    lo, hi = front.bounds()
    cam.fit_to(lo, hi, 1.2)
    eye, fwd = cam.eye(), (cam.target - cam.eye()).normalized()
    at = eye - fwd * 5.0 * (cam.target - cam.eye()).length()
    scene = Scene()
    scene.loose_mesh.add_face([at + V(0, 0, 0), at + V(2, 0, 0),
                               at + V(2, 2, 0), at + V(0, 2, 0)])
    scene.version += 1
    out["behind_eye"] = (list(VC._faces_of(scene)), cam, VC._Perspective(cam),
                         None)
    return out


# ---- Answers -----------------------------------------------------------------

def net(f) -> float:
    from built_core.vista_caras import _area
    return float(sum(_area(r) for r in f.rings))


def visible_record(out):
    """A visible-parts answer as the tests read it."""
    return [{"depth": float(f.depth), "net": net(f), "opacity": float(f.opacity),
             "rings": len(f.rings)} for f in out]


def geometry_record(geo):
    def arr(a):
        return np.asarray(a, float).tolist()
    rows = []
    for g in geo:
        if g is None:
            rows.append(None)
            continue
        world, rings2, depths, normal, back, near = g
        rows.append({"world": [arr(r) for r in world],
                     "rings": [arr(r) for r in rings2],
                     "depths": [arr(d) for d in depths],
                     "normal": arr(normal), "back": bool(back),
                     "near": None if near is None else [float(v) for v in near]})
    return rows


@lru_cache(maxsize=1)
def _golden() -> dict:
    return json.loads(gzip.decompress(GOLDEN.read_bytes()))


def python_visible(name):
    """The Python pass's visible faces of scene ``name`` (``VISIBLE``)."""
    return _golden()["visible"][name]


def python_geometry(name):
    """The Python pass's face geometry of case ``name``
    (:func:`geometry_cases`)."""
    return _golden()["geometry"][name]


def python_view_faces():
    """``view_faces`` of ``tests/test_faces_view_rust.py``'s scene with
    both Python passes: ``[(depth, layer, net)]`` of the faces above a
    square micrometre."""
    return _golden()["view_faces"]


def _write():
    from core import _native
    from built_core import vista_caras as VC
    data = {"visible": {}, "geometry": {}}
    for name, make in VISIBLE.items():
        data["visible"][name] = visible_record(VC._visible_parts_py(copies(make())))
    for name, (items, cam, proj, sp) in geometry_cases().items():
        data["geometry"][name] = geometry_record(
            VC._face_geometry_py(items, cam, proj, sp))
    from tests.test_faces_view_rust import _camera, _scene as model
    scene = model()
    cam = _camera(scene, False)
    keep = _native.native
    _native.native = None
    try:
        off = VC.view_faces(scene, cam)
    finally:
        _native.native = keep
    data["view_faces"] = [(float(f.depth), f.layer, abs(net(f)))
                          for f in off if abs(net(f)) > 1e-6]
    GOLDEN.write_bytes(gzip.compress(json.dumps(
        data, separators=(",", ":")).encode(), mtime=0))
    print("->", GOLDEN, GOLDEN.stat().st_size, "bytes")


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    _write()
