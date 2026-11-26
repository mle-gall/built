# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The dimension anchor's broad phase moved to Rust
(``model/anchor_broad.rs``) and must answer as ``core/dimension.py``'s
Python did at ``fff1954a``: ``_may_hold`` on deferred meshes (their source
points, the box kept between calls) and ``_AnchorPlacements``' entries and
candidates over seeded scenes: shared prototypes placed fewer and more than
32 times, turned, scaled, mirrored, nested, singular and projective
placements, classic groups, billboards, meshes that grow between queries,
deferred prototypes, survey coordinates. NumPy's ``(K, 3, 4) @ (4,)`` is
the pairwise two-lane sum on this Mac at every size measured, so every
answer is exact.

Regenerate only on purpose, against the reference implementation:
``python tests/test_anchor_broad_parity.py --write``."""
from __future__ import annotations

from tests.platform_golden import matches
import gzip
import json
import math
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "anchor_broad_golden.json.gz"


def _matrix(rng, M, survey=0.0):
    m = M()
    m.translate(rng.uniform(-50, 50) + survey, rng.uniform(-50, 50) + survey, rng.uniform(-5, 5))
    kind = rng.random()
    if kind < 0.5:
        m.rotate(rng.uniform(-180, 180), rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(0.1, 1))
    if kind < 0.3:
        m.scale(rng.choice((1.0, 2.5, 0.01, -1.0)), rng.uniform(0.5, 2), rng.uniform(0.5, 2))
    if kind > 0.97:
        m.scale(1.0, 0.0, 1.0)           # singular: dropped
    elif kind > 0.94:
        m = M(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0.01, 0, 0, 1)   # projective
    return m


def _mesh(rng, V, Mesh, n):
    m = Mesh()
    pts = [V(rng.uniform(-3, 3), rng.uniform(-3, 3), rng.uniform(0, 3)) for _ in range(n + 1)]
    for a, b in zip(pts, pts[1:]):
        m.add_edge(a, b)
    return m


def _deferred(rng, n, survey=0.0, dtype="float64"):
    import numpy as np
    from built_core.mesh import DeferredMesh, Mesh
    m = Mesh()
    pos = np.array([[rng.uniform(-3, 3) + survey, rng.uniform(-3, 3), rng.uniform(0, 3)]
                    for _ in range(n)], dtype=dtype)
    DeferredMesh.defer(m, lambda _m: None, 0, {"pos": pos} if n >= 0 else None)
    return m, pos


def _scene(seed):
    from built_core import Matrix4 as M, Vec3 as V
    from built_core.group import Group
    from built_core.mesh import Mesh
    from built_core.scene import Scene
    rng = random.Random(seed)
    scene = Scene()
    survey = 1e5 if seed % 5 == 4 else 0.0
    protos = [_mesh(rng, V, Mesh, rng.randint(1, 6)) for _ in range(3)]
    for i, proto in enumerate(protos):
        count = rng.choice((3, 31, 32, 40, 70)) if i else 40
        for _ in range(count):
            g = Group(proto)
            g.xform = _matrix(rng, M, survey)
            scene.groups.append(g)
    # Classic groups, nested components, a billboard.
    for _ in range(rng.randint(1, 4)):
        scene.groups.append(Group(_mesh(rng, V, Mesh, 3)))
    outer = Group(Mesh())
    outer.xform = _matrix(rng, M, survey)
    for _ in range(rng.randint(1, 40)):
        child = Group(protos[0])
        child.xform = _matrix(rng, M)
        outer.children.append(child)
    scene.groups.append(outer)
    bill = Group(_mesh(rng, V, Mesh, 2))
    bill.billboard = True
    scene.groups.append(bill)
    return scene, protos, rng


def _f(x):
    return repr(float(x)) if not math.isfinite(x) else float(x)


def _entry(scene, entry):
    root, placed, inverse = entry
    return [scene.groups.index(root), _uid_of(placed),
            None if inverse is None else [_f(v) for v in inverse.data()]]


_UIDS: dict = {}


def _uid_of(g):
    return _UIDS.setdefault(id(g), len(_UIDS))


def reference_candidates(entries, point, tolerance):
    """Evaluate the broad-phase contract on this host's actual inverses.

    Among other things this checks the threshold of 32 affine instances:
    a rounded homogeneous coefficient can change whether an inverse is
    exactly affine. Final resolved anchors are pinned on every platform.
    """
    from collections import Counter
    import numpy as np
    from built_core.mesh import DeferredMesh

    affine = [inv is not None and inv.isAffine() for _, _, inv in entries]
    counts = Counter(id(placed.mesh) for (_, placed, _), eligible in zip(entries, affine) if eligible)
    boxes = {}
    query = np.array([*point.toTuple(), 1.])
    for entry, eligible in zip(entries, affine):
        _, placed, inverse = entry
        mesh = placed.mesh
        if eligible and counts[id(mesh)] >= 32:
            if id(mesh) not in boxes:
                if type(mesh) is DeferredMesh:
                    source = vars(mesh).get("_deferred_source")
                    if not source:
                        yield entry
                        continue
                    positions = source["pos"]
                else:
                    positions = mesh._store.positions()
                boxes[id(mesh)] = ((np.min(positions, axis=0), np.max(positions, axis=0))
                                   if len(positions) else None)
            box = boxes[id(mesh)]
            if box is None:
                continue
            matrix = np.asarray(inverse.data()).reshape(4, 4).T[:3]
            local = matrix @ query
            slack = 4 * tolerance + 8 * np.finfo(np.float32).eps * (abs(matrix) @ abs(query))
            if np.any(local < box[0] - slack) or np.any(local > box[1] + slack):
                continue
        yield entry


def answers():
    from built_core import Vec3 as V
    import numpy as np
    import built_core as N
    from built_core import dimension as D
    out = {"may_hold": [], "scenes": []}
    rng = random.Random(20261125)
    for k in range(60):
        survey = 1e5 if k % 7 == 3 else 0.0
        n = rng.choice((0, 1, 5, 40))
        mesh, pos = _deferred(rng, n, survey, rng.choice(("float64", "float32")))
        row = []
        for q in range(12):
            tol = rng.choice((1e-4, 1e-3, 0.0))
            if len(pos) and q % 3 == 0:
                p = pos[rng.randrange(len(pos))].astype(np.float64)
                p = p + rng.choice((0.0, 3.9 * tol, 4.0 * tol, 4.1 * tol, -4.0 * tol))
            else:
                p = np.array([rng.uniform(-4, 4) + survey, rng.uniform(-4, 4), rng.uniform(-1, 4)])
            row.append(bool(N.dimension_may_hold(mesh, V(*p), tol)))
        out["may_hold"].append(row)
    # Not deferred, or deferred without source points: True.
    from built_core.mesh import DeferredMesh, Mesh
    plain = Mesh()
    bare = Mesh()
    DeferredMesh.defer(bare, lambda _m: None, 0, None)
    out["may_hold_plain"] = [bool(N.dimension_may_hold(plain, V(9, 9, 9), 1e-4)),
                             bool(N.dimension_may_hold(bare, V(9, 9, 9), 1e-4))]

    for seed in range(24):
        _UIDS.clear()
        scene, protos, srng = _scene(seed)
        if seed % 6 == 5:
            # A deferred prototype placed 40 times, and one without source.
            dm, dpos = _deferred(srng, 30)
            bare = Mesh()
            DeferredMesh.defer(bare, lambda _m: None, 0, None)
            from built_core.group import Group
            from built_core import Matrix4 as M
            for mesh in (dm, bare):
                for _ in range(40):
                    g = Group(mesh)
                    g.xform = _matrix(srng, M)
                    scene.groups.append(g)
        ap = N.AnchorPlacements(scene.groups)
        rec = {"entries": [_entry(scene, e) for e in ap.entries], "queries": []}
        pts = []
        for root in scene.groups:
            for v in () if type(root.mesh) is DeferredMesh else root.mesh.vertices[:2]:
                m = root.xform
                pts.append(m.map(v.position) if m is not None else V(v.position))
        srng.shuffle(pts)
        pts = pts[:25] + [V(srng.uniform(-60, 60), srng.uniform(-60, 60), srng.uniform(-6, 6))
                          for _ in range(10)]
        for i, p in enumerate(pts):
            if i == 18:
                protos[1].add_edge(V(7, 7, 7), V(8, 7, 7))   # a prototype grows
            tol = 1e-4 if i % 4 else 1e-2
            got = [_entry(scene, e) for e in ap.candidates(p, tol)]
            if sys.platform != "darwin":
                reference = [_entry(scene, e) for e in reference_candidates(ap.entries, p, tol)]
                assert got == reference, (seed, i)
            rec["queries"].append(got)
            anchor = D.resolve_vertex_anchor(scene, p, tol)
            rec["queries"].append(None if anchor is None else
                                  [_uid_of(g) for g in anchor.chain])
        out["scenes"].append(rec)
    return out


def test_anchor_broad_phase_answers_as_the_python_did():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(answers()))
    assert got["may_hold_plain"] == want["may_hold_plain"]
    assert got["may_hold"] == want["may_hold"]
    assert len(got["scenes"]) == len(want["scenes"])
    for i, (g, w) in enumerate(zip(got["scenes"], want["scenes"])):
        for index, (actual, expected) in enumerate(zip(g["entries"], w["entries"], strict=True)):
            assert matches(actual, expected), (i, index, actual, expected)
        for q, (a, b) in enumerate(zip(g["queries"], w["queries"], strict=True)):
            if sys.platform == "darwin" or q % 2:
                assert a == b, f"scene {i} query {q}"


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    GOLDEN.parent.mkdir(exist_ok=True)
    data = json.dumps(answers(), separators=(",", ":")).encode()
    GOLDEN.write_bytes(gzip.compress(data, mtime=0))
    print(GOLDEN, len(data))
