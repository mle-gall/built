# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Push/pull's native passes must leave every decision where Python puts it.

The orientation pass (``core.orient.orient_outward``, now
``ingetrazo_core.orient_decision``) and the push tool's searches over the
whole mesh (``core.push_scans``) run natively whenever the scans do; with
``scan_index._forced_off`` (``INGETRAZO_SCANS=0``) the original Python runs:
that is the oracle here.

- the pieces the native orientation reproduces bit for bit: Python's random
  stream, ``Face.normal()`` / ``centroid()`` / ``triangulate()``,
  ``_interior_probe``;
- ``orient_outward`` itself, differentially, on seeded scenes (boxes with
  faces wound either way, rooms sharing walls, holed and L-shaped faces,
  slivers, open sheets, nested and tilted solids), whole and scoped, on
  Python and store-backed meshes: the same faces turned over (returned and
  in the mesh), the same interior marks, the same paint; and the rollback of
  a flip through an open mesh;
- fixtures pinning decisions (which faces flip, which wall is interior, the
  push's inward limit, its edge classification, its levels);
- the push tool's prefilters against brute force (supersets in list order)
  and its exact native answers (the lateral exit, the cracked edges, the new
  elements by key, Python's set iteration order for the new faces);
- the engine certifier's push/pull sequences run twice, native and oracle,
  store on and off: the same model at the end.
"""
from __future__ import annotations

import os
import random
import sys

import numpy as np
import pytest
from tests import fuzz
from ingetrazo_core import Vec3 as QVector3D

import core._native as _native
from core import mesh as M
from core import orient as O
from core import push_scans, scan_index
from core import store_mesh as S

mod = _native.native
pytestmark = pytest.mark.skipif(
    mod is None or not hasattr(mod, "orient_decision")
    or _native.qt_hypot() is None,
    reason="native engine without the push/pull passes")


def V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


def _mesh(store: bool):
    m = M.Mesh()
    if store:
        S.become_store(m, mod.MeshStore(next(M._KEY_NAMESPACES),
                                        **S._weld_args()), [])
    return m


@pytest.fixture
def oracle(monkeypatch):
    """Switch between the native passes and the original Python."""
    def set_off(off: bool):
        monkeypatch.setattr(scan_index, "_forced_off", off)
        scan_index.forget()
    yield set_off
    scan_index.forget()


# ---- scenes ------------------------------------------------------------------

def _box(m, o, s, flip=(), tilt=0.0, drop=()):
    """A box of size ``s`` at ``o``, faces listed in a fixed order; the
    ones in ``flip`` wound inward, the ones in ``drop`` left out (an open
    box). ``tilt`` turns it about z."""
    import math
    c, sn = math.cos(tilt), math.sin(tilt)

    def P(dx, dy, dz):
        x, y = dx * s[0], dy * s[1]
        return V(o[0] + c * x - sn * y, o[1] + sn * x + c * y, o[2] + dz * s[2])
    pts = [P(dx, dy, dz) for dx in (0, 1) for dy in (0, 1) for dz in (0, 1)]
    quads = [(0, 2, 6, 4), (1, 5, 7, 3), (0, 4, 5, 1), (2, 3, 7, 6),
             (0, 1, 3, 2), (4, 6, 7, 5)]
    for k, q in enumerate(quads):
        if k in drop:
            continue
        lp = [pts[i] for i in q]
        if k in flip:
            lp.reverse()
        m.add_face(lp)


def _rooms(m, rng):
    """Two rooms sharing a wall (an interior partition), windings random."""
    xs = (0.0, 3.0, 6.0)
    for i in range(2):
        x0, x1 = xs[i], xs[i + 1]
        P = [V(x, y, z) for x in (x0, x1) for y in (0, 4) for z in (0, 2.5)]
        quads = [(0, 2, 6, 4), (1, 5, 7, 3), (0, 4, 5, 1), (2, 3, 7, 6),
                 (0, 1, 3, 2), (4, 6, 7, 5)]
        for q in quads:
            lp = [P[j] for j in q]
            if rng.random() < 0.3:
                lp.reverse()
            if i == 1 and q == (0, 1, 3, 2):
                continue            # the shared wall once only
            m.add_face(lp)


def _ring_box(m, rng):
    """A box whose top is a ring face (a hole) with a tube through: the
    probe of a holed face is its largest triangle's centroid."""
    h = 2.0
    outer = [V(10, 0, 0), V(14, 0, 0), V(14, 4, 0), V(10, 4, 0)]
    inner = [V(11.5, 1.5, 0), V(12.5, 1.5, 0), V(12.5, 2.5, 0), V(11.5, 2.5, 0)]
    up = V(0, 0, h)
    for z in (0.0, h):
        lift = V(0, 0, z)
        f = m.add_face([p + lift for p in outer],
                       hole_loops=[[p + lift for p in inner]])
        if rng.random() < 0.5:
            O.turn_face_over(f)
    for ring in (outer, inner):
        for i in range(4):
            a, b = ring[i], ring[(i + 1) % 4]
            lp = [a, b, b + up, a + up]
            if rng.random() < 0.4:
                lp.reverse()
            m.add_face(lp)


def _l_prism(m, rng):
    """An L-shaped prism: the caps' vertex average falls in the notch."""
    L = [V(20, 0), V(24, 0), V(24, 1), V(21, 1), V(21, 4), V(20, 4)]
    h = V(0, 0, 1.5)
    for cap in (L, [p + h for p in L]):
        lp = list(cap)
        if rng.random() < 0.5:
            lp.reverse()
        m.add_face(lp)
    for i in range(len(L)):
        a, b = L[i], L[(i + 1) % len(L)]
        lp = [a, b, b + h, a + h]
        if rng.random() < 0.3:
            lp.reverse()
        m.add_face(lp)


def _scene(store: bool, seed: int):
    rng = random.Random(seed)
    m = _mesh(store)
    kinds = rng.sample(["boxes", "rooms", "ring", "L", "nested", "sheet",
                        "sliver"], rng.randint(1, 4))
    for b in range(rng.randint(1, 3)):
        o = (rng.uniform(-5, 5), rng.uniform(-5, 5), rng.uniform(-1, 1))
        s = (rng.uniform(0.3, 3), rng.uniform(0.3, 3), rng.uniform(0.3, 3))
        _box(m, o, s, flip=[k for k in range(6) if rng.random() < 0.35],
             tilt=rng.uniform(0, 1.2) if rng.random() < 0.4 else 0.0)
    if "rooms" in kinds:
        _rooms(m, rng)
    if "ring" in kinds:
        _ring_box(m, rng)
    if "L" in kinds:
        _l_prism(m, rng)
    if "nested" in kinds:
        _box(m, (30, 0, 0), (4, 4, 4), flip=[k for k in range(6)
                                             if rng.random() < 0.3])
        _box(m, (31, 1, 1), (1, 1, 1), flip=[k for k in range(6)
                                             if rng.random() < 0.5])
    if "sheet" in kinds:
        m.add_face([V(40, 0, 0), V(41, 0, 0), V(41, 1, 0.3)])
    if "sliver" in kinds:
        m.add_face([V(50, 0, 0), V(51, 0, 0), V(52, 0, 0)])
    return m


def _state(m):
    return [([v.position.toTuple() for v in f.loop],
             [[v.position.toTuple() for v in h] for h in f.hole_loops],
             bool(f.interior), sorted(f.attrs.items()))
            for f in m.faces]


def _index(m, faces):
    keys = [f.key for f in m.faces]
    return [keys.index(f.key) for f in faces]


# ---- the pieces ------------------------------------------------------------------

def test_random_stream_is_pythons():
    for seed in (0, 1, 12345, 2 ** 32 - 1, 2 ** 32, 2 ** 40 + 7, 2 ** 63 + 5):
        r = random.Random(seed)
        assert mod.py_random_stream(seed, 1500) == [r.random()
                                                    for _ in range(1500)]


@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("seed", range(6))
def test_face_geometry_is_pythons(store, seed):
    m = _scene(store, seed)
    s = scan_index.scan(m, fresh=True)
    n, c, tris = mod.scan_face_geometry(s.native, _native.qt_hypot(),
                                        sys.version_info >= (3, 12))
    probes = mod.scan_face_probes(s.native, _native.qt_hypot(),
                                  sys.version_info >= (3, 12))
    for i, f in enumerate(m.faces):
        assert n[i] == f.normal().toTuple()
        assert c[i] == f.centroid().toTuple()
        assert [tuple(p) for p in tris[i]] == [
            tuple(q.toTuple() for q in t) for t in f.triangulate()]
        assert probes[i] == O._interior_probe(f).toTuple()


# ---- orient_outward, differentially -----------------------------------------------

def _orient_both(oracle, store, seed, scope):
    out = []
    for off in (True, False):
        oracle(off)
        m = _scene(store, seed)
        faces = list(m.faces)
        only = None
        if scope == "one":
            only = [faces[seed % len(faces)]]
        elif scope == "some":
            only = faces[::5]
        got = O.orient_outward(m, only=only)
        out.append((_index(m, got), _state(m)))
    return out


@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("scope", ["all", "one", "some"])
@pytest.mark.parametrize("seed", fuzz.sample(range(160), 4, "orient_outward"))
def test_orient_outward_native_matches_python(oracle, store, scope, seed):
    py, nat = _orient_both(oracle, store, seed, scope)
    assert nat == py


@pytest.mark.slow
@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("seed", range(160))
def test_orient_outward_native_matches_python_fuzz(oracle, store, seed):
    for scope in ("all", "one", "some"):
        py, nat = _orient_both(oracle, store, seed, scope)
        assert nat == py, (seed, scope)


def _tube(m, rng, o, length, w, h, tilt):
    """A tube open at both ends with a partition across it: its walls in
    two lengths meeting at the partition, which shares their edges. Rays
    from the partition (and from a wall wound inward) leave through the
    open ends or hit a wall depending on their direction."""
    import math
    c, sn = math.cos(tilt), math.sin(tilt)

    def P(x, y, z):
        return V(o[0] + c * x - sn * y, o[1] + sn * x + c * y, o[2] + z)
    cut = rng.uniform(0.3, 0.7) * length
    ring = [(0, 0), (w, 0), (w, h), (0, h)]
    for x0, x1 in ((0.0, cut), (cut, length)):
        for i in range(4):
            (y0, z0), (y1, z1) = ring[i], ring[(i + 1) % 4]
            lp = [P(x0, y0, z0), P(x0, y1, z1), P(x1, y1, z1), P(x1, y0, z0)]
            if rng.random() < 0.3:
                lp.reverse()
            m.add_face(lp)
    lp = [P(cut, y, z) for y, z in ring]
    if rng.random() < 0.5:
        lp.reverse()
    m.add_face(lp)


def _cluttered(store, seed):
    """Open tubes with partitions, and boxes, crowding each other: parity
    through an open surface depends on the direction, so with a wide
    jitter the votes (the interior rounds ask every face of the scope)
    hang on the exact rays — which partitions read as interior, and every
    later face's offset in the random stream with them."""
    rng = random.Random(seed)
    m = _mesh(store)
    for _ in range(4):
        _tube(m, rng, (rng.uniform(0, 3), rng.uniform(0, 3), rng.uniform(0, 3)),
              rng.uniform(2, 4), rng.uniform(0.5, 1.5), rng.uniform(0.5, 1.5),
              rng.uniform(0, 1.5))
    for _ in range(3):
        o = (rng.uniform(0, 4), rng.uniform(0, 4), rng.uniform(0, 4))
        sz = (rng.uniform(0.5, 3), rng.uniform(0.5, 3), rng.uniform(0.5, 3))
        _box(m, o, sz, flip=[k for k in range(6) if rng.random() < 0.4],
             tilt=rng.uniform(0, 0.8))
    return m


@pytest.mark.parametrize("rays,jitter", [(7, 0.5), (5, 1.0), (9, 0.5)])
@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("seed", range(8))
def test_orient_jitter_dependent_votes_match(oracle, monkeypatch, rays,
                                             jitter, store, seed):
    """With rays spread wide, a face's vote depends on its exact jittered
    directions — so on Python's random stream, on each face's offset in it
    and on the float32 ray arithmetic: the native pass must still agree."""
    monkeypatch.setattr(O, "_RAYS", rays)
    monkeypatch.setattr(O, "_JITTER", jitter)
    out = []
    for off in (True, False):
        oracle(off)
        m = _cluttered(store, seed)
        got = O.orient_outward(m)
        out.append((_index(m, got), _state(m)))
    assert out[1] == out[0]


def test_wide_jitter_votes_do_depend_on_the_stream():
    """The test above has teeth: on these scenes another seed gives
    another decision somewhere."""
    hyp = (_native.qt_hypot(), sys.version_info >= (3, 12))
    differ = 0
    for seed in range(4):
        m = _cluttered(False, seed)
        s = scan_index.scan(m, fresh=True)
        a = mod.orient_decision(s.native, None, 12345, 7, 0.5, *hyp)
        b = mod.orient_decision(s.native, None, 777, 7, 0.5, *hyp)
        differ += a != b
    assert differ >= 2


def test_orient_fixtures(oracle):
    """Which faces flip and which wall is a partition, pinned."""
    for off in (True, False):
        oracle(off)
        for store in (False, True):
            m = _mesh(store)
            _box(m, (0, 0, 0), (2, 3, 4), flip=(1, 4))
            flipped = O.orient_outward(m)
            assert sorted(_index(m, flipped)) == [1, 4]
            assert all(QVector3D.dotProduct(f.normal(), f.centroid() - V(1, 1.5, 2))
                       > 0 for f in m.faces)
            m = _mesh(store)
            _rooms(m, random.Random(3))
            O.orient_outward(m)
            interior = [f.centroid().toTuple() for f in m.faces if f.interior]
            assert interior == [(3.0, 2.0, 1.25)]


def _frame_box(m, o, ex, ey, ez):
    """A box on the axes ``ex``, ``ey``, ``ez`` (any directions) at ``o``."""
    P = [o + ex * dx + ey * dy + ez * dz
         for dx in (0, 1) for dy in (0, 1) for dz in (0, 1)]
    for q in [(0, 2, 6, 4), (1, 5, 7, 3), (0, 4, 5, 1), (2, 3, 7, 6),
              (0, 1, 3, 2), (4, 6, 7, 5)]:
        m.add_face([P[i] for i in q])


def _poking_boxes(m, angle, size):
    """A 2 m box and one turned ``angle`` degrees about its bottom front
    edge (sharing it: one edge-connected solid) so it pokes into the first,
    beside an open sheet. Both are wound outward, but parity reads faces
    inside the other box inward: the flips would leave the solid's shared
    edges less consistent."""
    import math
    a = math.radians(angle)
    _box(m, (0, 0, 0), (2, 2, 2))
    _frame_box(m, V(0, 0, 0), V(2, 0, 0),
               V(0, math.cos(a), math.sin(a)) * size,
               V(0, -math.sin(a), math.cos(a)) * size)
    m.add_face([V(9, 0, 0), V(10, 0, 0), V(10, 1, 0)])


@pytest.mark.parametrize("angle,size", [(30, 0.5), (45, 1.0), (60, 1.5)])
def test_a_flip_that_worsens_an_open_solid_is_never_made(oracle, monkeypatch,
                                                         angle, size):
    """Through an open mesh, flips that leave a solid's shared edges less
    consistent are taken back (Python) / never made (native): the same
    model either way, and the oracle does take the rollback here."""
    real = O._incoherent_edges
    seen = []
    results = []
    for off in (True, False):
        oracle(off)
        for store in (False, True):
            m = _mesh(store)
            _poking_boxes(m, angle, size)
            m.faces[7].attrs = {"color": [1, 2, 3, 255]}
            if off:
                monkeypatch.setattr(O, "_incoherent_edges",
                                    lambda fs: seen.append(real(fs))
                                    or seen[-1])
            got = O.orient_outward(m, only=[m.faces[0]])
            monkeypatch.setattr(O, "_incoherent_edges", real)
            if off:
                assert seen[-1] > seen[-2]          # worse: rolled back
                assert type(m) is S.StoreMesh
            results.append((_index(m, got), _state(m)))
    assert all(r == results[0] for r in results)


def test_orient_python_rollback_on_a_store_mesh_stays_in_the_store(
        oracle, monkeypatch):
    """The Python pass's rollback reverses a store-backed face's rings with
    one store op (it used to write the loop back and materialise)."""
    monkeypatch.setattr(S.StoreMesh, "_materialise",
                        lambda mesh: pytest.fail("orientation materialised the mesh"))
    oracle(True)
    m = _mesh(True)
    _box(m, (3, 0, 0), (1, 1, 1))
    m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
    victim = m.faces[0]
    loop = [v.key for v in victim.loop]
    real = O._face_side_state

    def misread(f, *a, **k):
        return "inward" if f == victim else real(f, *a, **k)
    monkeypatch.setattr(O, "_face_side_state", misread)
    assert O.orient_outward(m, only=[victim]) == []
    assert type(m) is S.StoreMesh
    assert [v.key for v in m.faces[0].loop] == loop


# ---- the push tool's searches ------------------------------------------------------

def _random_points_on_edges(m, rng, n):
    out = []
    faces = list(m.faces)
    for _ in range(n):
        f = rng.choice(faces)
        lp = f.vertices
        i = rng.randrange(len(lp))
        a, b = lp[i], lp[(i + 1) % len(lp)]
        t0, t1 = sorted((rng.choice([0.0, rng.random()]),
                         rng.choice([1.0, rng.random()])))
        out.append((a + (b - a) * t0, a + (b - a) * t1))
    return out


@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("seed", range(5))
def test_segment_prefilter_is_a_superset(store, seed):
    from core.topology import _face_all_edges, _key, _segment_on_face_boundary
    rng = random.Random(seed)
    m = _scene(store, seed)
    faces = list(m.faces)
    for a, b in _random_points_on_edges(m, rng, 25):
        got = push_scans.faces_on_segment(m, a, b)
        keys = [g.key for g in got]
        assert keys == sorted(keys, key=[f.key for f in faces].index)
        want = [g.key for g in faces
                if _segment_on_face_boundary(a, b, g)
                or frozenset((_key(a), _key(b))) in _face_all_edges(g)]
        assert set(want) <= set(keys)


@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("seed", range(5))
def test_parallel_overlap_prefilter_and_lateral_exit(store, seed, oracle):
    """The parallel-overlap candidates hold every face the inward limit and
    the levels can take; the lateral exit is the Python loop's, exactly."""
    from core.orient import _ray_triangle
    from core.push_pull import _loops_overlap_2d, _project_loop_2d
    rng = random.Random(seed)
    m = _scene(store, seed)
    O.orient_outward(m)
    faces = list(m.faces)
    for base in rng.sample(faces, min(6, len(faces))):
        n = base.normal().normalized()
        base_xy = _project_loop_2d(base.vertices, n)
        got = {g.key for g in push_scans.parallel_overlapping(m, n, base_xy)}
        for g in faces:
            gn = g.normal().normalized()
            if (abs(QVector3D.dotProduct(gn, n)) >= 0.999 and _loops_overlap_2d(
                    base_xy, _project_loop_2d(g.vertices, n))):
                assert g.key in got
        d = -n
        samples = [base.centroid()] + [p + (base.centroid() - p) * 1e-3
                                       for p in base.vertices]
        want = None
        for g in faces:
            if g == base or g.interior:
                continue
            if abs(QVector3D.dotProduct(g.normal().normalized(), n)) > 0.999:
                continue
            for tri in g.triangulate():
                for q in samples:
                    t = _ray_triangle(q, d, tri)
                    if t is not None and t > 1e-4 and (want is None
                                                       or t < want):
                        want = t
        assert push_scans.lateral_exit(m, base, samples, d, n) == (True, want)


@pytest.mark.parametrize("store", [False, True])
def test_cracked_edges_and_nearby_vertices(store):
    from core.topology import _key
    m = _scene(store, 2)
    m.add_face([V(60, 0, 0), V(61, 0, 0), V(61, 1, 0)])
    edges = list(m.edges)
    for near in (None, (59.5, -1, -1, 60.5, 1, 1), (-100, -100, -100, 0, 0, 0)):
        got = push_scans.cracked_edges(m, near)

        def inside(p):
            return (near[0] <= p.x() <= near[3] and near[1] <= p.y() <= near[4]
                    and near[2] <= p.z() <= near[5])
        want = [e.key for e in edges if len(e.faces) == 1
                and (near is None or inside(e.a) or inside(e.b))]
        assert [e.key for e in got] == want
    pts = [f.vertices[0] for f in list(m.faces)[::3]]
    keys = {_key(p) for p in pts}
    got = [v.key for v in push_scans.vertices_at_keys(m, pts)]
    want = [v.key for v in m.vertices if _key(v.position) in keys]
    assert set(want) <= set(got)
    assert got == [v.key for v in m.vertices if v.key in set(got)]


def test_new_faces_keep_pythons_set_order():
    """A set of views is a set of their keys: the difference built by key
    walks the same table and comes out in the same order."""
    rng = random.Random(7)
    m = _mesh(True)
    _box(m, (0, 0, 0), (1, 1, 1))
    for _ in range(40):
        before = set(m.faces)
        bkeys = push_scans.live_keys(m, "face")
        for _ in range(rng.randint(1, 30)):
            if rng.random() < 0.7 or len(m.faces) < 8:
                o = V(rng.uniform(-9, 9), rng.uniform(-9, 9), rng.uniform(-9, 9))
                m.add_face([o, o + V(0.1, 0, 0), o + V(0, 0.1, 0)])
            else:
                m.remove_face(rng.choice(list(m.faces)))
        want = [f.key for f in set(m.faces) - before]
        got = push_scans.set_minus(m, "face", push_scans.live_keys(m, "face"),
                                   set(bkeys))
        assert got is not None and [f.key for f in got] == want
        mark = push_scans.mark(m)
        for _ in range(rng.randint(0, 5)):
            o = V(rng.uniform(-9, 9), rng.uniform(-9, 9), 20)
            m.add_face([o, o + V(0.1, 0, 0), o + V(0, 0.1, 0)])
        snap_keys = set(mark.keys("edge"))
        assert [e.key for e in push_scans.made_since(m, mark, "edge")] == [
            e.key for e in m.edges if e.key not in snap_keys]


# ---- push/pull fixtures and the engine certifier ------------------------------------

def _push_state(store, oracle, off, build, pick, dist):
    from tests.test_fuzz_engine import _StubVP, History, Scene
    from tools.pushpull import PushPullTool
    oracle(off)
    scene = Scene()
    if store:
        S.become_store(scene.mesh, mod.MeshStore(
            next(M._KEY_NAMESPACES), **S._weld_args()), [])
    hist = History(scene)
    build(scene.mesh)
    O.orient_outward(scene.mesh)
    face = pick(scene.mesh)
    tool = PushPullTool()
    tool.base_face = face
    tool.dragging = True
    target = tool._target_scene(scene)
    O.orient_outward(target.mesh, only=(face,))
    tool._anchor = face.centroid()
    tool._normal = tool._outward_normal(face, target.mesh)
    tool._attached, tool._prism_cap = tool._classify_base(target)
    tool._cap_positions = tool._cap_loop_positions(face)
    tool._compute_inward_limit(target)
    tool._compute_plane_levels(target)
    decided = (tool._attached, tool._prism_cap, tool._limit_in,
               list(tool._plane_levels),
               [v.position.toTuple() for v in tool._cap_vertices(target)])
    tool.extrusion = dist
    tool._clamp_extrusion()
    tool._commit(_StubVP(scene, hist))
    return decided, _state(scene.mesh), hist.last_error


def _step(m):
    """A stepped solid: a 4×4×1 slab with a 2×2×1 block on a corner, and a
    floor slab under it — parallel faces at several levels."""
    _box(m, (0, 0, 0), (4, 4, 1))
    _box(m, (0, 0, 1), (2, 2, 1))
    _box(m, (0, 0, -3), (4, 4, 0.5))


def _top(m, z):
    return next(f for f in m.faces
                if abs(f.centroid().z() - z) < 1e-6 and f.normal().z() > 0.5)


@pytest.mark.parametrize("store", [False, True])
def test_push_fixtures(oracle, store):
    """The push's decisions pinned, and the same with the oracle: the
    inward limit (the slab below), the edge classification of a cap
    (prism) and of a face set into a wider top (not), the levels."""
    cases = [
        ("box top, pushed in", lambda m: _box(m, (0, 0, 0), (2, 2, 3)),
         lambda m: _top(m, 3.0), -1.0,
         lambda d: d[0] and d[1] and abs(d[2] - 3.0) < 1e-6),
        ("block top, pulled", _step, lambda m: _top(m, 2.0), 0.5,
         lambda d: d[0] and abs(d[2] - 1.0) < 1e-6),
    ]
    for name, build, pick, dist, check in cases:
        nat = _push_state(store, oracle, False, build, pick, dist)
        py = _push_state(store, oracle, True, build, pick, dist)
        assert nat == py, name
        assert check(nat[0]), (name, nat[0])


def _certifier_run(monkeypatch, scenario, seed, store):
    import tests.test_fuzz_engine as FE
    made = []

    class _Scene(FE.Scene):
        def __init__(self, *a, **k):
            super().__init__(*a, **k)
            if store:
                S.become_store(self.mesh, mod.MeshStore(
                    next(M._KEY_NAMESPACES), **S._weld_args()), [])
            made.append(self)
    monkeypatch.setattr(FE, "Scene", _Scene)
    FE.run_sequence(scenario, seed)
    return FE._scene_fingerprint(made[0])


def _certify_both(monkeypatch, oracle, scenario, seed, store):
    oracle(True)
    py = _certifier_run(monkeypatch, scenario, seed, store)
    oracle(False)
    nat = _certifier_run(monkeypatch, scenario, seed, store)
    assert nat == py


@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("scenario", ["cube", "prism", "plan", "group"])
@pytest.mark.parametrize("seed", range(5))
def test_engine_certifier_native_matches_oracle(monkeypatch, oracle, scenario,
                                                seed, store):
    """The certifier's sequences (draws on faces, pushes of every kind,
    undo/redo, its invariants) run native and oracle: the same model."""
    _certify_both(monkeypatch, oracle, scenario, seed, store)


@pytest.mark.slow
@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("scenario", ["cube", "prism", "plan", "group"])
@pytest.mark.parametrize("seed", range(5, 60))
def test_engine_certifier_native_matches_oracle_fuzz(monkeypatch, oracle,
                                                     scenario, seed, store):
    _certify_both(monkeypatch, oracle, scenario, seed, store)


def test_native_orientation_is_used(monkeypatch):
    """Guard against the native pass silently falling back: on a store mesh
    with the scans on, the Python pass never runs."""
    m = _mesh(True)
    _box(m, (0, 0, 0), (1, 1, 1), flip=(2,))
    monkeypatch.setattr(O, "_face_components", lambda *_: (_ for _ in ()).throw(
        AssertionError("the Python pass ran")))
    # A replaced helper makes the pass Python: undo that for this check.
    monkeypatch.setitem(O._REPRODUCED, "_face_components", O._face_components)
    assert len(O.orient_outward(m)) == 1
    assert os.environ.get("INGETRAZO_SCANS") != "0"
