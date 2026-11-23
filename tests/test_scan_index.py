# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""``core.scan_index``: the drawing commands' whole-mesh searches as native
prefilters must leave every decision where the full Python loops put it.

Two layers:

- the prefilters against brute force: every query returns a superset (in
  list order) of what the predicate it stands in for can accept, on Python
  and store-backed meshes;
- the commands themselves, differentially: the same seeded scene is built
  twice and the same random draws, rectangles, erasures, deletions, undos
  and redos are made on both, one with the scans and one with them forced
  off (the original loops, the oracle). After every step both meshes must
  be the same, element for element and in list order: positions, edge ends,
  flags and faces, face loops, holes and attrs, and the history's verdict.
  Fixtures pin the planner's decisions on the cases that matter (a line
  across a face, vertex to vertex, a rectangle inside a face and against
  its edge, an erased split line, a deleted face with a hole).
"""
from __future__ import annotations

import os
import random

import numpy as np
import pytest
from ingetrazo_core import Vec3 as QVector3D
from ingetrazo_core import Vec3 as V

import core._native as _native
from core import mesh as M
from core import scan_index
from core import store_mesh as S

mod = _native.available()
pytestmark = pytest.mark.skipif(
    mod is None or not hasattr(mod, "MeshScan"),
    reason="native module without MeshScan")


# ---- scenes ------------------------------------------------------------------

def _grid_points(n, flat_rows):
    """Quads of an n x n grid; rows below ``flat_rows`` lie on z=0 (one
    plane: coplanar neighbours), the others on z = g(i) + h(j) (planar
    quads, no two neighbours coplanar)."""
    def P(i, j):
        if j < flat_rows:
            return (float(i), float(j), 0.0)
        return (float(i), float(j), 0.02 * i * i + 0.013 * (j - flat_rows) ** 2)
    pts = []
    for i in range(n):
        for j in range(n):
            pts += [P(i, j), P(i + 1, j), P(i + 1, j + 1), P(i, j + 1)]
    return pts


def _scene(store: bool, n: int = 6, flat_rows: int = 3, seed: int = 0,
           flat: bool = False):
    """A deterministic scene: a grid of quads (bulk: store-backed when
    ``store``), a box beside it, free edges, a holed face and a curve.
    ``flat``: everything on z=0 (no box): a 2D drawing, where the whole-
    flat gate and the planar rebuilds take over."""
    if flat:
        flat_rows = n + 1          # the last row of points too
    from core.edits import build_add_edge, build_add_edges
    from core.history import AddFaceCommand, History
    from core.scene import Scene
    was = os.environ.get("INGETRAZO_STORE")
    os.environ["INGETRAZO_STORE"] = "1" if store else "0"
    try:
        scene = Scene()
        pts = _grid_points(n, flat_rows)
        scene.mesh.add_faces_bulk(np.asarray(pts, float),
                                  np.full(n * n, 4), np.ones(n * n, int))
    finally:
        if was is None:
            del os.environ["INGETRAZO_STORE"]
        else:
            os.environ["INGETRAZO_STORE"] = was
    assert (type(scene.mesh) is S.StoreMesh) == store
    h = History(scene)
    rng = random.Random(seed)
    # A box standing off the grid.
    x0, y0 = n + 2.0, 1.0
    for z in (() if flat else (0.0, 1.5)):
        c = [V(x0, y0, z), V(x0 + 2, y0, z), V(x0 + 2, y0 + 2, z), V(x0, y0 + 2, z)]
        h.execute(build_add_edges(scene, [(c[i], c[(i + 1) % 4]) for i in range(4)],
                                  detect_faces=False, extra=[AddFaceCommand(c)]))
    for i in (() if flat else range(4)):
        a = [V(x0, y0, 0), V(x0 + 2, y0, 0), V(x0 + 2, y0 + 2, 0), V(x0, y0 + 2, 0)]
        p, q = a[i], a[(i + 1) % 4]
        h.execute(build_add_edge(scene, p, p + V(0, 0, 1.5)))
        h.execute(build_add_edge(scene, p + V(0, 0, 1.5), q + V(0, 0, 1.5)))
    # Free edges and a holed face (a rectangle inside a flat quad).
    for _ in range(3):
        a = V(rng.uniform(-3, -1), rng.uniform(0, n),
              0.0 if flat else rng.uniform(0, 2))
        h.execute(build_add_edge(scene, a, a + V(rng.uniform(0.2, 1), 0, 0)))
    c = [V(1.25, 0.25, 0), V(1.75, 0.25, 0), V(1.75, 0.75, 0), V(1.25, 0.75, 0)]
    h.execute(build_add_edges(scene, [(c[i], c[(i + 1) % 4]) for i in range(4)],
                              detect_faces=False, extra=[AddFaceCommand(c)],
                              plane_rebuild=True))
    # A curve on the flat part.
    ring = [V(3.5 + 0.3 * np.cos(t), 1.5 + 0.3 * np.sin(t), 0.0)
            for t in np.linspace(0, 2 * np.pi, 9)[:-1]]
    segs = [(ring[i], ring[(i + 1) % len(ring)]) for i in range(len(ring))]
    h.execute(build_add_edges(scene, segs, detect_faces=True))
    scene.mesh.tag_curve(ring)
    return scene


def _fingerprint(m):
    """Everything a draw decides, by list index (keys differ between two
    scenes; curve ids are renumbered in order of appearance)."""
    Vs, Es, Fs = list(m.vertices), list(m.edges), list(m.faces)
    vid = {v.key: i for i, v in enumerate(Vs)}
    fid = {f.key: i for i, f in enumerate(Fs)}
    curves: dict = {}

    def cv(c):
        return None if c is None else curves.setdefault(c, len(curves))

    def r(v):
        return vid.get(v.key, ("unlisted", v.position.toTuple()))
    return (
        [v.position.toTuple() for v in Vs],
        [(r(e.v0), r(e.v1), bool(e.soft), bool(e.hidden), cv(e.curve),
          e.layer, [fid.get(f.key, -1) for f in e.faces]) for e in Es],
        [([r(v) for v in f.loop], [[r(v) for v in h] for h in f.hole_loops],
          sorted((k, repr(v)) for k, v in f.attrs.items()), bool(f.interior))
         for f in Fs],
    )


# ---- random operations ---------------------------------------------------------

def _face_point(rng, f):
    tris = f.triangulate()
    if not tris:
        return QVector3D(f.vertices[0])
    a, b, c = rng.choice(tris)
    u, w = rng.random(), rng.random()
    if u + w > 1:
        u, w = 1 - u, 1 - w
    return a + (b - a) * u + (c - a) * w



def _points_on(rng, m):
    """Two points a user could click: corners, edge midpoints, interior
    points of one face (mostly the same face, for chord splits)."""
    f = rng.choice(list(m.faces))
    loop = f.vertices

    def one():
        k = rng.random()
        if k < 0.35:
            return QVector3D(rng.choice(loop))
        if k < 0.7:
            i = rng.randrange(len(loop))
            a, b = loop[i], loop[(i + 1) % len(loop)]
            return (a + b) * 0.5
        if k < 0.9:
            return _face_point(rng, f)
        g = rng.choice(list(m.faces))
        return _face_point(rng, g)
    a, b = one(), one()
    return a, b


def _rect_on(rng, m):
    from core.triangulate import plane_axes
    f = rng.choice(list(m.faces))
    n = f.normal()
    u, w = plane_axes(n)
    c = _face_point(rng, f)
    if rng.random() < 0.4:
        c = QVector3D(rng.choice(f.vertices))      # against a corner
    du, dv = rng.uniform(0.05, 0.7), rng.uniform(0.05, 0.7)
    return [c, c + u * du, c + u * du + w * dv, c + w * dv]


def _op(rng, scene, h, kind):
    """One step; returns a label. Draws use the real command builders."""
    from core.edits import build_add_edge, build_add_edges
    from core.history import (AddFaceCommand, DeleteEdgesCommand,
                              EraseSelectionCommand)
    m = scene.mesh
    if kind == "line":
        a, b = _points_on(rng, m)
        if (a - b).length() < 1e-6:
            return "skip"
        h.execute(build_add_edge(scene, a, b, detect_faces=True))
        return "line"
    if kind == "rect":
        c = _rect_on(rng, m)
        h.execute(build_add_edges(scene, [(c[i], c[(i + 1) % 4]) for i in range(4)],
                                  detect_faces=False, extra=[AddFaceCommand(c)],
                                  plane_rebuild=True))
        return "rect"
    if kind == "erase":
        es = list(m.edges)
        if not es:
            return "skip"
        picked = [rng.choice(es) for _ in range(rng.choice([1, 1, 1, 2]))]
        h.execute(EraseSelectionCommand(picked, []))
        return "erase"
    if kind == "delete":
        fs = list(m.faces)
        if not fs:
            return "skip"
        h.execute(EraseSelectionCommand([], [rng.choice(fs)]))
        return "delete"
    if kind == "cascade":
        es = list(m.edges)
        if not es:
            return "skip"
        h.execute(DeleteEdgesCommand([rng.choice(es)]))
        return "cascade"
    if kind == "undo":
        h.undo()
        return "undo"
    if kind == "redo":
        h.redo()
        return "redo"
    raise AssertionError(kind)


_KINDS = (["line"] * 6 + ["rect"] * 3 + ["erase"] * 3 + ["delete"] * 2
          + ["cascade"] + ["undo"] * 2 + ["redo"])


class _KeyHashes:
    """Python elements hash by the local part of their key while active.

    The Python kernel follows set order in places (the loop a coplanar
    merge starts from), and a set of objects hashed by address iterates
    differently in two scenes built alike: two runs with the scans OFF
    already disagree. By key, both scenes number their elements alike
    and iterate alike, so any difference left is the scans'."""

    # The materialised objects and the store's views hash by their full
    # key, whose namespace part (another per scene) steers set probing on
    # collisions: same treatment, so a view and its object still meet.
    _CLASSES = (M.Vertex, M.Edge, M.Face, S._MVertex, S._MEdge, S._MFace,
                S._ViewMixin)

    def __enter__(self):
        self._old = [c.__dict__.get("__hash__") for c in self._CLASSES]
        for c in self._CLASSES:
            c.__hash__ = _local_hash
        return self

    def __exit__(self, *exc):
        for c, h in zip(self._CLASSES, self._old):
            if h is None:
                del c.__hash__
            else:
                c.__hash__ = h


def _local_hash(self):
    return self.key & ((1 << 40) - 1)


def run_differential(seed: int, store: bool, n_ops: int, n: int = 6,
                     flat: bool = False) -> int:
    """The seeded sequence on two copies, scans on and off; returns the
    number of steps compared."""
    with _KeyHashes():
        return _run_differential(seed, store, n_ops, n, flat)


def _run_differential(seed: int, store: bool, n_ops: int, n: int,
                      flat: bool) -> int:
    from core.history import History
    on = _scene(store, n=n, seed=seed, flat=flat)
    off_was = scan_index._forced_off
    scan_index._forced_off = True
    try:
        off = _scene(store, n=n, seed=seed, flat=flat)
    finally:
        scan_index._forced_off = off_was
    assert _fingerprint(on.mesh) == _fingerprint(off.mesh)
    h_on, h_off = History(on), History(off)
    steps = 0
    kinds = random.Random(seed * 7919 + 1)
    for step in range(n_ops):
        kind = kinds.choice(_KINDS)
        r_on, r_off = random.Random(seed * 1000 + step), random.Random(seed * 1000 + step)
        label = _op(r_on, on, h_on, kind)
        scan_index._forced_off = True
        try:
            _op(r_off, off, h_off, kind)
        finally:
            scan_index._forced_off = off_was
        ctx = f"seed {seed} step {step} {label} store={store}"
        assert h_on.last_error == h_off.last_error, ctx
        assert h_on.last_rejected == h_off.last_rejected, ctx
        assert _fingerprint(on.mesh) == _fingerprint(off.mesh), ctx
        steps += 1
    return steps


@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("seed", range(6))
def test_commands_decide_as_the_full_scans(seed, store):
    assert run_differential(seed, store, n_ops=24) == 24


@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("seed", range(4))
def test_commands_decide_as_the_full_scans_on_a_flat_drawing(seed, store):
    assert run_differential(seed, store, n_ops=20, flat=True) == 20


@pytest.mark.slow
@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("seed", range(6, 46))
def test_commands_decide_as_the_full_scans_random(seed, store):
    run_differential(seed, store, n_ops=60, n=8, flat=seed % 4 == 0)


# ---- fixtures: the planner's decisions, pinned ------------------------------------

@pytest.fixture(autouse=True)
def _deterministic_sets():
    with _KeyHashes():
        yield


def _pair(store):
    from core.history import History
    on = _scene(store)
    was = scan_index._forced_off
    scan_index._forced_off = True
    try:
        off = _scene(store)
    finally:
        scan_index._forced_off = was
    return (on, History(on)), (off, History(off))


def _both(pair, fn):
    (on, h_on), (off, h_off) = pair
    fn(on, h_on)
    was = scan_index._forced_off
    scan_index._forced_off = True
    try:
        fn(off, h_off)
    finally:
        scan_index._forced_off = was
    assert h_on.last_error is None and h_off.last_error is None
    assert _fingerprint(on.mesh) == _fingerprint(off.mesh)
    return on


def _P(i, j, flat_rows=3):
    """Grid point (i, j) of :func:`_scene`."""
    z = 0.0 if j < flat_rows else 0.02 * i * i + 0.013 * (j - flat_rows) ** 2
    return V(float(i), float(j), z)


def _find(m, pts):
    from core.history import _find_face_by_loop
    return _find_face_by_loop(m, pts)


def _face_at(m, *corners):
    from core.history import _find_face_by_loop
    return _find_face_by_loop(m, [V(*c) for c in corners])


@pytest.mark.parametrize("store", [False, True])
def test_line_across_a_face_splits_that_face(store):
    from core.edits import build_add_edge
    pair = _pair(store)
    nf = len(pair[0][0].mesh.faces)

    # Across the non-flat quad (4,4)-(5,5): midpoints of its two x-edges.
    def draw2(scene, h):
        h.execute(build_add_edge(scene, (_P(4, 4) + _P(5, 4)) * 0.5,
                                 (_P(4, 5) + _P(5, 5)) * 0.5, detect_faces=True))
    m = pair[0][0].mesh
    quad = [_P(4, 4), _P(5, 4), _P(5, 5), _P(4, 5)]
    assert _find(m, quad) is not None
    on = _both(pair, draw2)
    assert len(on.mesh.faces) == nf + 1
    assert _find(on.mesh, quad) is None


@pytest.mark.parametrize("store", [False, True])
def test_line_vertex_to_vertex_splits_a_flat_quad(store):
    from core.edits import build_add_edge
    pair = _pair(store)
    nf = len(pair[0][0].mesh.faces)
    on = _both(pair, lambda s, h: h.execute(
        build_add_edge(s, V(2, 1, 0), V(3, 2, 0), detect_faces=True)))
    assert len(on.mesh.faces) == nf + 1


@pytest.mark.parametrize("store", [False, True])
def test_rectangle_inside_and_against_an_edge(store):
    from core.edits import build_add_edges
    from core.history import AddFaceCommand

    def rect(c):
        def fn(s, h):
            h.execute(build_add_edges(s, [(c[i], c[(i + 1) % 4]) for i in range(4)],
                                      detect_faces=False, extra=[AddFaceCommand(c)],
                                      plane_rebuild=True))
        return fn
    pair = _pair(store)
    nf = len(pair[0][0].mesh.faces)
    on = _both(pair, rect([V(4.2, 0.2, 0), V(4.6, 0.2, 0), V(4.6, 0.6, 0), V(4.2, 0.6, 0)]))
    assert len(on.mesh.faces) == nf + 1            # a hole and its fill
    on = _both(pair, rect([V(0, 2.2, 0), V(0.5, 2.2, 0), V(0.5, 2.6, 0), V(0, 2.6, 0)]))
    assert len(on.mesh.faces) >= nf + 2            # carved against the edge


@pytest.mark.parametrize("store", [False, True])
def test_erasing_the_split_line_reunites_the_face(store):
    from core.edits import build_add_edge
    from core.history import EraseSelectionCommand
    pair = _pair(store)
    nf = len(pair[0][0].mesh.faces)
    _both(pair, lambda s, h: h.execute(
        build_add_edge(s, V(2, 1, 0), V(3, 2, 0), detect_faces=True)))

    def erase(s, h):
        m = s.mesh
        e = m.find_edge(m.vertex_at(V(2, 1, 0)), m.vertex_at(V(3, 2, 0)))
        h.execute(EraseSelectionCommand([e], []))
    on = _both(pair, erase)
    assert len(on.mesh.faces) == nf


@pytest.mark.parametrize("store", [False, True])
def test_delete_the_holed_face_and_a_crease_edge(store):
    from core.history import EraseSelectionCommand
    pair = _pair(store)
    nf = len(pair[0][0].mesh.faces)

    def delete(s, h):
        f = _face_at(s.mesh, (1, 0, 0), (2, 0, 0), (2, 1, 0), (1, 1, 0))
        assert f is not None and f.hole_loops
        h.execute(EraseSelectionCommand([], [f]))
    on = _both(pair, delete)
    assert len(on.mesh.faces) == nf - 1

    def crease(s, h):
        # The box's bottom edge: bottom and side meet square, so the erase
        # takes both faces.
        m = s.mesh
        e = m.find_edge(m.vertex_at(V(8, 1, 0)), m.vertex_at(V(10, 1, 0)))
        assert e is not None and len(e.faces) == 2
        h.execute(EraseSelectionCommand([e], []))
    on = _both(pair, crease)
    assert len(on.mesh.faces) == nf - 3

    def gentle(s, h):
        # Between two grid quads whose normals agree within 0.999: merged.
        m = s.mesh
        e = m.find_edge(m.vertex_at(_P(4, 4)), m.vertex_at(_P(5, 4)))
        assert e is not None and len(e.faces) == 2
        h.execute(EraseSelectionCommand([e], []))
    on = _both(pair, gentle)
    assert len(on.mesh.faces) == nf - 4


# ---- the prefilters against brute force ---------------------------------------------

def _random_mesh(rng, store):
    pts = []
    rings = []
    for _ in range(rng.randrange(20, 60)):
        c = np.array([rng.uniform(-5, 5) for _ in range(3)])
        k = rng.choice([3, 4, 5])
        u = np.array([rng.uniform(-1, 1) for _ in range(3)])
        w = np.array([rng.uniform(-1, 1) for _ in range(3)])
        ring = [c + u * np.cos(t) + w * np.sin(t)
                for t in np.linspace(0, 2 * np.pi, k + 1)[:-1]]
        if rng.random() < 0.2:
            ring[0] = ring[0] + np.array([0, 0, rng.uniform(-0.5, 0.5)])   # warped
        rings.append(len(ring))
        pts += [tuple(p) for p in ring]
    was = os.environ.get("INGETRAZO_STORE")
    os.environ["INGETRAZO_STORE"] = "1" if store else "0"
    try:
        m = M.Mesh()
        m.add_faces_bulk(np.asarray(pts, float), np.asarray(rings),
                         np.ones(len(rings), int))
    finally:
        if was is None:
            del os.environ["INGETRAZO_STORE"]
        else:
            os.environ["INGETRAZO_STORE"] = was
    for _ in range(10):
        m.add_edge(V(*[rng.uniform(-5, 5) for _ in range(3)]),
                   V(*[rng.uniform(-5, 5) for _ in range(3)]))
    return m


def _pbox(pts):
    xs, ys, zs = zip(*[(p.x(), p.y(), p.z()) for p in pts])
    return (min(xs), min(ys), min(zs)), (max(xs), max(ys), max(zs))


def _meets(b, lo, hi):
    return all(b[1][k] >= lo[k] and b[0][k] <= hi[k] for k in range(3))


@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("seed", range(8))
def test_prefilters_are_supersets_in_list_order(seed, store):
    from core.history import RebuildPlaneFacesCommand
    from core.topology import loop_inside_face
    rng = random.Random(seed)
    m = _random_mesh(rng, store)
    sc = scan_index.scan(m)
    assert sc is not None and sc.store_backed == store
    faces, edges = list(m.faces), list(m.edges)
    fkeys = [f.key for f in faces]
    for _ in range(30):
        a = V(*[rng.uniform(-6, 6) for _ in range(3)])
        b = a + V(*[rng.uniform(-3, 3) for _ in range(3)])
        lo, hi = _pbox([a, b])
        got = sc.faces_meeting((lo, hi))
        want = [i for i, f in enumerate(faces) if _meets(_pbox(f.vertices), lo, hi)]
        assert got == want
        got = sc.edges_meeting((lo, hi))
        want = [i for i, e in enumerate(edges) if _meets(_pbox([e.a, e.b]), lo, hi)]
        assert got == want
        assert [f.key for f in sc.faces(sc.faces_meeting((lo, hi)))] == [
            fkeys[i] for i in sc.faces_meeting((lo, hi))]
    # Containment: every face a loop can be strictly inside is held.
    from core.history import _faces_holding
    for f in faces:
        c = f.centroid()
        loop = [c + (x - c) * 0.6 for x in f.vertices]
        held = {x.key for x in _faces_holding(m, loop)}
        for g in faces:
            if loop_inside_face(g, loop, outside_holes=True):
                assert g.key in held
    # Plane queries: every face / edge the rebuild's test accepts.
    for f in faces[:10]:
        cmd = RebuildPlaneFacesCommand(f.vertices[0], f.normal())
        on = {x.key for x in cmd._near_plane(m, "face")}
        for g in faces:
            if all(cmd._on_plane(v.position) for v in g.loop):
                assert g.key in on
        on_e = {x.key for x in cmd._near_plane(m, "edge")}
        for e in edges:
            if cmd._on_plane(e.a) and cmd._on_plane(e.b):
                assert e.key in on_e


def test_cycle_adjacency_is_the_full_one():
    """The planner's lazy adjacency lists equal the full build's, node by
    node, over a scene with removed edges and plan pieces."""
    from core.edits import _ScanEdges
    from core.geometry import Edge as GEdge
    from core.topology import _cycle_adjacency, _key
    scene = _scene(True)
    m = scene.mesh
    sc = scan_index.scan(m)
    sim = _ScanEdges(sc)
    full = list(m.edges)
    rng = random.Random(3)
    for e in [rng.choice(full) for _ in range(5)]:
        picked = sim.pick((e.a, e.b))
        hit = [x for x in picked if x == e][0]
        if hit in sim:
            sim.remove(hit)
            full = [x for x in full if x != e]
    for _ in range(4):
        a = V(rng.choice(full).a)
        b = V(rng.uniform(0, 5), rng.uniform(0, 5), 0)
        sim.append(GEdge(a, b))
        full.append(GEdge(a, b))
    ka, kb = _key(full[0].a), _key(full[0].b)
    lazy = sim.cycle_adjacency(ka, kb)
    ref = _cycle_adjacency(full, ka, kb)
    for u, lst in ref.items():
        got = lazy.get(u)
        assert [(k, p.toTuple()) for k, p in got] == [(k, p.toTuple()) for k, p in lst]
    assert (_key(V(99, 99, 99)) in lazy) is False


@pytest.mark.parametrize("store", [False, True])
@pytest.mark.parametrize("seed", range(12))
def test_whole_drawing_plane_is_coplanar_plane(seed, store):
    """The flat-drawing gate over the scan answers as ``coplanar_plane`` on
    every vertex: points at the tolerance's edge, a drawing whose first
    points coincide or line up, a tilted plane far from the origin."""
    from core.arrangement import coplanar_plane
    from core.edits import _whole_drawing_plane
    from core.scene import Scene
    rng = random.Random(seed)
    tilt = V(rng.uniform(-0.3, 0.3), rng.uniform(-0.3, 0.3), 1.0).normalized()
    u = V.crossProduct(tilt, V(1, 0, 0)).normalized()
    w = V.crossProduct(tilt, u)
    far = V(rng.choice([0, 300, -2500]), rng.choice([0, 700]), 0)
    start = rng.choice(["plain", "same", "line"])
    pts = []
    for k in range(rng.randrange(6, 40)):
        if start == "same" and k < 3:
            a, b = 0.0, 0.0
        elif start == "line" and k < 5:
            a, b = float(k), 0.0
        else:
            a, b = rng.uniform(-5, 5), rng.uniform(-5, 5)
        off = rng.choice([0.0, 0.0, 0.99e-4, 1e-4, 1.01e-4, -1e-4, 3e-5])
        pts.append(far + u * a + w * b + tilt * off)
    was = os.environ.get("INGETRAZO_STORE")
    os.environ["INGETRAZO_STORE"] = "1" if store else "0"
    try:
        scene = Scene()
        n = len(pts) // 3 * 3
        scene.mesh.add_faces_bulk(
            np.asarray([(p.x(), p.y(), p.z()) for p in pts[:n]], float),
            np.full(n // 3, 3), np.ones(n // 3, int))
    finally:
        if was is None:
            del os.environ["INGETRAZO_STORE"]
        else:
            os.environ["INGETRAZO_STORE"] = was
    drawn = [pts[-1] + u * 0.5, pts[-1] + w * 0.5 + tilt * rng.choice([0, 2e-4])]
    want = coplanar_plane([v.position for v in scene.mesh.vertices] + drawn)
    got = _whole_drawing_plane(scene, drawn)
    assert (got is None) == (want is None)
    if got is not None:
        assert got[0].toTuple() == want[0].toTuple()
        assert got[1].toTuple() == want[1].toTuple()
