# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The synthetic scenes of the HLR parity fixtures (tests/test_hlr_parity.py).

Every case is rebuilt from a seed, so the golden files hold only what the
drawing pass produced: segments, world points, kinds and layers. The
goldens in ``tests/fixtures/hlr_parity/`` were written by the Python
drawing pass of ``core/hlr.py`` (integ/built 60e017a8) before its loop
moved to Rust."""
from __future__ import annotations

from functools import lru_cache
import gzip
import json
from pathlib import Path
import math
import random

import numpy as np
from built_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D

from built_core.camera import OrbitCamera
from built_core.composition import MarcoVista, apply_frame_camera
from built_core.scene import Scene
from built_core.section import SectionPlane

VIEWS = ("top", "front", "right", "iso")


@lru_cache(maxsize=1)
def _boxes():
    # Freeze the rotated input corners as well as the drawing. Regenerating
    # the inputs with host libm perturbs coincident projected edges.
    path = Path(__file__).parent / "fixtures/hlr_parity/boxes.json.gz"
    return json.loads(gzip.decompress(path.read_bytes()))


def boxes_scene(seed: int) -> Scene:
    data = _boxes()[str(seed)]
    scene = Scene()
    for face in data["faces"]:
        scene.mesh.add_face([QVector3D(*point) for point in face])
    if "section_z" in data:
        scene.section_planes = [SectionPlane(
            QVector3D(0, 0, data["section_z"]), QVector3D(0, 0, -1), active=True)]
    return scene


def camera(scene, view: str, rot_deg: float = 0.0) -> OrbitCamera:
    cam = OrbitCamera()
    if rot_deg:
        frame = MarcoVista(view_key=f"std:{view}", scale_n=50.0, w_mm=120.0,
                           h_mm=90.0, rot_deg=rot_deg)
        apply_frame_camera(cam, frame, saved_view=None, scene=scene)
        return cam
    cam.set_view(view)
    lo, hi = scene.bounds()
    cam.fit_to(lo, hi)
    return cam


class _Bare:
    """A scene with no section: what ``hlr_drawing`` reads besides the
    geometry it is handed."""

    show_section_cuts = False


def soft_geometry(seed: int, tags: bool):
    """Array geometry as ``Viewport.hlr_geometry`` returns it: a faceted
    cylinder (soft sides, its seams), a box beside it and an open fin,
    with a tag per triangle and edge when ``tags``."""
    rng = random.Random(1000 + seed)
    n = rng.randint(10, 24)
    r, h = rng.uniform(0.5, 1.5), rng.uniform(1.0, 3.0)
    cx, cy = rng.uniform(-1, 1), rng.uniform(-1, 1)
    ring = [(cx + r * math.cos(2 * math.pi * k / n), cy + r * math.sin(2 * math.pi * k / n))
            for k in range(n)]
    tris, hard, soft, soft_n = [], [], [], []
    tt, ht, st = [], [], []
    for k in range(n):
        (x0, y0), (x1, y1) = ring[k], ring[(k + 1) % n]
        a, b, c, d = (x0, y0, 0), (x1, y1, 0), (x1, y1, h), (x0, y0, h)
        tris += [(a, b, c), (a, c, d)]
        tt += ["Muros", "Muros"]
        hard += [(a, b), (d, c)]                      # bottom and top rims
        ht += ["Muros", "Muros"]
        mid0 = 2 * math.pi * (k - 0.5) / n
        mid1 = 2 * math.pi * (k + 0.5) / n
        soft.append(((x0, y0, 0), (x0, y0, h)))
        soft_n.append(((math.cos(mid0), math.sin(mid0), 0), (math.cos(mid1), math.sin(mid1), 0)))
        st.append("Muros")
    bx, by = cx + r + rng.uniform(0.2, 1.0), cy + rng.uniform(-0.5, 0.5)
    s = rng.uniform(0.4, 1.2)
    corners = [(bx, by), (bx + s, by), (bx + s, by + s), (bx, by + s)]
    for k in range(4):
        (x0, y0), (x1, y1) = corners[k], corners[(k + 1) % 4]
        a, b, c, d = (x0, y0, 0), (x1, y1, 0), (x1, y1, s), (x0, y0, s)
        tris += [(a, b, c), (a, c, d)]
        tt += ["Mobiliario", "Mobiliario"]
        hard += [(a, b), (d, c), (a, d)]
        ht += ["Mobiliario", None, "Mobiliario"]
    top = [(x, y, s) for x, y in corners]
    tris += [(top[0], top[1], top[2]), (top[0], top[2], top[3])]
    tt += [None, None]
    fin = [(cx - r - 0.6, cy, 0.0), (cx - r - 0.1, cy, 0.0), (cx - r - 0.35, cy, 1.2)]
    tris.append(tuple(fin))
    tt.append("Mobiliario")
    for k in range(3):                                 # an open face: its rim is soft
        soft.append((fin[k], fin[(k + 1) % 3]))
        soft_n.append(((0, -1, 0), (float("nan"),) * 3))
        st.append("Mobiliario")
    geometry = (np.asarray(tris, dtype=float), np.asarray(hard, dtype=float),
                np.asarray(soft, dtype=float), np.asarray(soft_n, dtype=float))
    if tags:
        geometry += (np.asarray(tt, dtype=object), np.asarray(ht, dtype=object),
                     np.asarray(st, dtype=object))
    return geometry


def soft_camera(view: str) -> OrbitCamera:
    from built_core import Vec3
    cam = OrbitCamera()
    cam.set_view(view)
    cam.fit_to(Vec3(-3, -3, 0), Vec3(4, 3, 3))
    return cam


def cases():
    """``(name, run)`` for every fixture; ``run(**kw)`` → HlrDrawing."""
    from built_core.hlr import hlr_drawing
    out = []
    for seed in range(12):
        for view in VIEWS:
            for hidden in (False, True):
                def run(seed=seed, view=view, hidden=hidden):
                    scene = boxes_scene(seed)
                    return hlr_drawing(scene, camera(scene, view), hidden=hidden)
                out.append((f"boxes-{seed}-{view}-{'hidden' if hidden else 'seen'}", run))
    for seed in (1, 4, 7):
        for view in ("top", "iso"):
            def run(seed=seed, view=view):
                scene = boxes_scene(seed)
                return hlr_drawing(scene, camera(scene, view, rot_deg=30.0), hidden=True)
            out.append((f"rotated-{seed}-{view}", run))
    for seed in range(4):
        for view in ("front", "iso", "top"):
            for tags in (False, True):
                for hidden in (False, True):
                    def run(seed=seed, view=view, tags=tags, hidden=hidden):
                        return hlr_drawing(_Bare(), soft_camera(view),
                                           geometry=soft_geometry(seed, tags),
                                           layers=tags, hidden=hidden)
                    name = f"soft-{seed}-{view}-{'tags' if tags else 'plain'}-{'hidden' if hidden else 'seen'}"
                    out.append((name, run))
    for seed in (0, 3):
        def run(seed=seed):
            scene = boxes_scene(seed)
            return hlr_drawing(scene, camera(scene, "iso"), profiles=False)
        out.append((f"noprofiles-{seed}", run))
    return out


def canonical(drawing):
    """The drawing as sorted rows ``(kind, layer, x0, y0, x1, y1, world…)``
    so two passes that emit the same segments in another order compare
    equal."""
    layers = drawing.layers
    rows = []
    for i in range(len(drawing.segs)):
        tag = "" if layers is None or layers[i] is None else str(layers[i])
        rows.append((int(drawing.kinds[i]), tag, *map(float, drawing.segs[i]),
                     *map(float, np.asarray(drawing.world[i]).reshape(-1))))
    # Sorted on coordinates rounded to 0.1 mm: two passes that differ in the
    # last bits must not swap neighbours whose x0 agree.
    rows.sort(key=lambda r: (r[0], r[1], *(round(v, 4) for v in r[2:6]), r[2:]))
    return rows
