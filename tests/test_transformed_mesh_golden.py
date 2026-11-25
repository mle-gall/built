# SPDX-License-Identifier: GPL-3.0-or-later
"""``core.group.transformed_mesh`` and ``world_mesh`` must build what the
Python at ``9a8150f4`` built: seeded meshes (quads, triangles, faces with
holes, soft and curved edges, painted, textured with and without a fitted
``uvw``, small and past the bulk threshold) through seeded matrices
(rotations, scales, mirrors, survey offsets), and seeded components with
nested placements. Each result is compared as geometry: every face's rings
as world corners (to 1e-9), its attributes (``uvw`` to 1e-9), every edge's
ends with its soft and curve flags. Positions are NumPy's ``pts @ rot.T +
trans``, whose order can change with the number of rows (Accelerate), so
corners compare to 1e-9 m, not bit for bit.
``tests/fixtures/transformed_mesh_golden.json.gz`` holds the answers;
regenerate only on purpose: ``python tests/test_transformed_mesh_golden.py --write``."""
from __future__ import annotations

import gzip
import json
import random
import sys
from pathlib import Path

import pytest

GOLDEN = Path(__file__).parent / "fixtures" / "transformed_mesh_golden.json.gz"
CASES = 40


def r9(x):
    return round(float(x), 9) + 0.0


def look(mesh):
    faces = []
    for f in mesh.faces:
        rings = [f.loop, *(f.hole_loops or ())]
        attrs = {}
        for k, v in (f.attrs or {}).items():
            if k == "texture" and isinstance(v, dict):
                v = {kk: ([r9(x) for x in vv] if kk == "uvw" else vv) for kk, vv in v.items()}
            attrs[k] = v
        faces.append([[[r9(c) for c in v.position_tuple()] for v in ring] for ring in rings]
                     + [json.dumps(attrs, sort_keys=True, default=repr)])
    edges = sorted([sorted([[r9(c) for c in e.v0.position_tuple()], [r9(c) for c in e.v1.position_tuple()]]),
                    bool(e.soft), e.curve is not None] for e in mesh.edges)
    return {"faces": sorted(faces, key=repr), "edges": edges}


def mesh_of(rng, n):
    from ingetrazo_core import Vec3 as V
    from core.mesh import Mesh
    m = Mesh()
    for i in range(n):
        x, y = (i % 30) * 2.0, (i // 30) * 2.0
        kind = rng.random()
        if kind < 0.15:
            f = m.add_face([V(x, y, 0), V(x + 1.5, y, 0), V(x + 1.5, y + 1.5, 0), V(x, y + 1.5, 0)],
                           [[V(x + 0.5, y + 0.5, 0), V(x + 1, y + 0.5, 0), V(x + 1, y + 1, 0), V(x + 0.5, y + 1, 0)]])
        elif kind < 0.4:
            f = m.add_face([V(x, y, 0), V(x + 1, y, 0), V(x, y + 1, 0.5)])
        else:
            f = m.add_face([V(x, y, 0), V(x + 1, y, 0), V(x + 1, y + 1, 0), V(x, y + 1, 0)])
        r = rng.random()
        if r < 0.2:
            f.attrs["texture"] = {"path": "wood.png", "sw": 0.5, "sh": 0.25}
        elif r < 0.35:
            f.attrs["texture"] = {"path": "brick.png", "sw": 1.0, "sh": 1.0,
                                  "uvw": [1.0, 0.0, 0.0, 0.25, 0.0, 1.0, 0.0, -0.5]}
        elif r < 0.5:
            f.attrs["color"] = [0.1, 0.2, 0.3]
        if rng.random() < 0.1:
            f.attrs["layer"] = "Wood"
    for e in m.edges[::7]:
        e.soft = True
    return m


def matrix(rng):
    from ingetrazo_core import Matrix4
    t = Matrix4()
    t.translate(rng.choice([0.0, 1.25, 1e5]), rng.uniform(-3, 3), rng.uniform(-1, 1))
    t.rotate(rng.uniform(0, 360), rng.random(), rng.random(), rng.random() + 0.2)
    if rng.random() < 0.3:
        t.scale(rng.choice([-1.0, 2.0, 0.5]), 1.0, rng.choice([1.0, -1.0]))
    return t


def answers(seed):
    from ingetrazo_core import Matrix4
    from core.group import Group, world_mesh
    from ingetrazo_core import group_transformed_mesh as transformed_mesh
    rng = random.Random(seed)
    n = rng.choice([4, 20, 300])
    src = mesh_of(rng, n)
    out = {"transformed": look(transformed_mesh(src, matrix(rng)))}
    top = Group(src, name="top")
    top.xform = matrix(rng) if rng.random() < 0.5 else Matrix4()
    kids = []
    for k in range(rng.randint(1, 3)):
        c = Group(mesh_of(rng, rng.choice([3, 40])), name=f"c{k}")
        c.xform = matrix(rng)
        kids.append(c)
    top.adopt(kids)
    out["world"] = look(world_mesh(top))
    return out


@pytest.mark.parametrize("seed", range(CASES))
def test_transformed_and_world_meshes_build_as_the_python_did(seed):
    assert answers(seed) == _golden()[seed]


_CACHE = []


def _golden():
    if not _CACHE:
        with gzip.open(GOLDEN, "rt") as f:
            _CACHE.append(json.load(f))
    return _CACHE[0]


@pytest.mark.parametrize("n", [1, 6, 400])
def test_a_copy_carries_soft_flat_and_curves_at_any_size(n):
    """One build at any size: the small-mesh walk copied the flat flag and
    the vectorized copy did not; both carry soft, flat and curves now."""
    from ingetrazo_core import Matrix4, Vec3 as V
    from ingetrazo_core import group_transformed_mesh as transformed_mesh
    from core.mesh import Mesh
    m = Mesh()
    for i in range(n):
        m.add_face([V(i, 0, 0), V(i + 1, 0, 0), V(i + 1, 1, 0), V(i, 1, 0)])
    edges = list(m.edges)
    edges[0].soft = True
    edges[1].flat = True
    t = Matrix4()
    t.translate(1.0, 2.0, 0.0)
    t.rotate(90.0, 0, 0, 1)
    out = transformed_mesh(m, t)

    def flags(mesh, mapped):
        return sorted((tuple(sorted(tuple(round(c, 9) + 0.0 for c in mapped(p)) for p in (e.v0.position_tuple(), e.v1.position_tuple()))),
                       bool(e.soft), bool(e.flat)) for e in mesh.edges)
    def through(p):
        q = t.map(V(*p))
        return (q.x(), q.y(), q.z())
    assert flags(out, lambda p: p) == flags(m, through)
    assert sum(e.flat for e in out.edges) == 1 and sum(e.soft for e in out.edges) == 1


if __name__ == "__main__":
    if "--write" in sys.argv:
        raw = json.dumps([answers(seed) for seed in range(CASES)]).encode()
        with open(GOLDEN, "wb") as fh, gzip.GzipFile(fileobj=fh, mode="wb", mtime=0) as gz:
            gz.write(raw)
        print("wrote", GOLDEN)
