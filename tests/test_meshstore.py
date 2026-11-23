# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Rust ``MeshStore`` skeleton (docs/meshstore-design.md) against the
Python it will replace, with ``core/mesh.py`` as the oracle:

- built from a format-3 block it IS ``lazy_mesh.plan`` and the mesh the
  bulk path builds, index for index (vertex order, edge order and
  direction, flags, corners, each edge's faces with multiplicity);
- ``move_vertex`` / ``place_vertex`` leave the weld registry, the
  displaced queue, ``registers`` and ``vertex_at`` exactly as
  ``Mesh.move_vertex`` / ``place_vertex`` do, on points crowded at weld
  cell borders and moved onto each other;
- ``snapshot`` / ``restore`` / ``same_as`` behave as ``capture_state`` /
  ``restore_state`` / ``==`` (keys kept, ``_displaced`` cleared).

Nothing in the app uses the store yet; these tests are its contract."""
from __future__ import annotations

import random

import numpy as np
import pytest
from ingetrazo_core import Vec3 as V

import core._native as _native
from core import lazy_mesh
from core import mesh as M
from core.scene import Scene
from formats import igz
from tests.test_deferred_mesh import _meshes, _random_scene

mod = _native.available()
pytestmark = pytest.mark.skipif(
    mod is None or not hasattr(mod, "MeshStore"),
    reason="native module without MeshStore")

SCALE = M._KEY_SCALE
TOL2 = (10.0 ** -M._KEY_DECIMALS) ** 2


@pytest.fixture
def binary(monkeypatch):
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    monkeypatch.setattr(igz, "_BULK_MIN", 1)
    monkeypatch.setattr(_native, "wanted", lambda: True)


def _ns():
    return next(M._KEY_NAMESPACES)


def store_of_source(src):
    """The store a deferred mesh's source builds (what the facade will do
    instead of ``DeferredMesh.defer``)."""
    names = src["meta"].get("edge_layers") or []
    edges = np.ascontiguousarray(src["edges"], dtype=np.int64).reshape(-1, 2)
    opt = (lambda a, t: None if a is None
           else np.ascontiguousarray(a, dtype=t))
    return mod.MeshStore.from_block(
        _ns(), np.ascontiguousarray(src["pos"], dtype=np.float32).reshape(-1, 3),
        edges, opt(src["flags"], np.uint8), opt(src["curves"], np.int64),
        opt(src["layers"], np.int64), list(names),
        np.ascontiguousarray(src["rings"], dtype=np.int64),
        np.ascontiguousarray(src["sizes"], dtype=np.int64),
        np.ascontiguousarray(src["counts"], dtype=np.int64),
        opt(src.get("face_attr"), np.int64),
        sequential=bool(src.get("sequential", False)), scale=SCALE, tol2=TOL2)


def compare_plan(plan, store):
    a = store.arrays()
    names = store.layer_names()
    assert a["pos"].tolist() == plan.pos.tolist()
    assert a["edges"].tolist() == plan.edges.tolist()
    assert ((a["flags"] & 1) > 0).tolist() == plan.soft.tolist()
    assert ((a["flags"] & 2) > 0).tolist() == plan.hidden.tolist()
    assert [None if c < 0 else c for c in a["curve"].tolist()] == plan.curve
    assert [None if k < 0 else names[k] for k in a["layer"].tolist()] == \
        plan.layer
    assert a["corners"].tolist() == plan.corners.tolist()
    assert a["ring_sizes"].tolist() == plan.ring_sizes.tolist()
    assert a["ring_counts"].tolist() == plan.ring_counts.tolist()
    assert a["ef_start"].tolist() == plan.ef_start.tolist()
    assert a["ef_face"].tolist() == plan.ef_face.tolist()


def compare_mesh(store, m):
    """The store against a built ``Mesh``, element for element."""
    a = store.arrays()
    names = store.layer_names()
    vid = {v.key: i for i, v in enumerate(m.vertices)}
    fid = {f.key: i for i, f in enumerate(m.faces)}
    assert a["pos"].tolist() == [list(v.position.toTuple())
                                 for v in m.vertices]
    assert a["edges"].tolist() == [[vid[e.v0.key], vid[e.v1.key]]
                                   for e in m.edges]
    assert [(bool(f & 1), bool(f & 2)) for f in a["flags"].tolist()] == \
        [(bool(e.soft), bool(e.hidden)) for e in m.edges]
    assert [None if c < 0 else c for c in a["curve"].tolist()] == \
        [e.curve for e in m.edges]
    assert [None if k < 0 else names[k] for k in a["layer"].tolist()] == \
        [e.layer for e in m.edges]
    assert a["corners"].tolist() == [vid[v.key] for f in m.faces
                                     for ring in [f.loop] + f.hole_loops
                                     for v in ring]
    got = [a["ef_face"][a["ef_start"][i]:a["ef_start"][i + 1]].tolist()
           for i in range(len(m.edges))]
    assert got == [[fid[f.key] for f in e.faces] for e in m.edges]
    # Vertex.edges as sets of edge indices.
    eid = {e.key: i for i, e in enumerate(m.edges)}
    adj = [set() for _ in m.vertices]
    for i, (p, q) in enumerate(a["edges"].tolist()):
        adj[p].add(i)
        adj[q].add(i)
    assert adj == [{eid[e.key] for e in v.edges} for v in m.vertices]


def compare_weld(store, m):
    """Registry, displaced queue, registers() — by vertex list index."""
    vid = {v.key: i for i, v in enumerate(m.vertices)}
    cells, ids = store.registry()
    got = {tuple(c): i for c, i in zip(cells.tolist(), ids.tolist())}
    assert got == {k: vid[v.key] for k, v in m._registry.items()}
    disp = {k: v for k, v in store.displaced().items()}
    assert disp == {k: [vid[w.key] for w in q]
                    for k, q in m._displaced.items()}
    assert store.positions().tolist() == [list(v.position.toTuple())
                                          for v in m.vertices]
    for i, v in enumerate(m.vertices):
        assert store.registers(store.key_at("vertex", i)) == m.registers(v)


def build_mesh(pos, edges, flags, curves, layers, names, rings, sizes,
               counts):
    """``formats/igz._build_geometry`` on an empty mesh."""
    m = M.Mesh()
    vobjs, inv = m.bulk_weld(np.asarray(pos, dtype=np.float64))
    emap = None
    if len(edges):
        eflags = [(bool(f & 1), c if c >= 0 else None,
                   names[k] if k >= 0 else None, bool(f & 2))
                  for f, c, k in zip(flags.tolist(), curves.tolist(),
                                     layers.tolist())]
        emap = m.add_edges_welded(vobjs, inv[edges[:, 0]], inv[edges[:, 1]],
                                  eflags)
    if len(counts):
        m.add_faces_welded(vobjs, inv[rings], sizes, counts, edge_map=emap)
    return m


def crowded_block(rng, n_points=200, n_edges=150, n_faces=60):
    """Points two cells apart or a hair across a cell border (so the weld
    probes run), edges and rings drawn over them: duplicates, pairs
    welded shut, rings of 1 and 2 corners, consecutive welded corners."""
    base = [[rng.randrange(-4, 4) * 0.5 for _ in range(3)]
            for _ in range(n_points // 3)]
    pts = []
    for _ in range(n_points):
        b = rng.choice(base)
        pts.append([c + rng.choice([0, 4.9e-5, 5.1e-5, -5e-5, 1e-6, 9.9e-5,
                                    0.25]) for c in b])
    pos = np.asarray(pts, dtype=np.float32)
    edges = np.asarray([[rng.randrange(n_points), rng.randrange(n_points)]
                        for _ in range(n_edges)], dtype=np.int64)
    flags = np.asarray([rng.choice([0, 0, 1, 2, 3]) for _ in edges],
                       dtype=np.uint8)
    curves = np.asarray([rng.choice([-1, -1, 4, 9]) for _ in edges],
                        dtype=np.int64)
    names = ["tag-a", "tag-b"]
    layers = np.asarray([rng.choice([-1, -1, 0, 1]) for _ in edges],
                        dtype=np.int64)
    rings, sizes, counts = [], [], []
    for _ in range(n_faces):
        nr = rng.choice([1, 1, 1, 2, 3])
        counts.append(nr)
        for _ in range(nr):
            s = rng.choice([1, 2, 3, 4, 4, 5])
            sizes.append(s)
            rings.extend(rng.randrange(n_points) for _ in range(s))
    return (pos, edges, flags, curves, layers, names,
            np.asarray(rings, dtype=np.int64), np.asarray(sizes, np.int64),
            np.asarray(counts, np.int64))


def store_of_block(blk, sequential=False):
    pos, edges, flags, curves, layers, names, rings, sizes, counts = blk
    return mod.MeshStore.from_block(
        _ns(), pos, edges, flags, curves, layers, names, rings, sizes,
        counts, None, sequential=sequential, scale=SCALE, tol2=TOL2)


# ---- Construction --------------------------------------------------------

@pytest.mark.parametrize("seed", range(10))
def test_store_is_the_plan_and_the_built_mesh(binary, tmp_path, seed):
    path = tmp_path / "doc.igz"
    igz.save_scene(_random_scene(random.Random(seed)), path)
    scene = Scene()
    igz.load_into(scene, path)
    n = 0
    for m in _meshes(scene):
        plan = lazy_mesh.plan_of(m)
        assert plan is not None
        store = store_of_source(m.__dict__["_deferred_source"])
        compare_plan(plan, store)
        fa = store.arrays()["face_attr"]
        assert fa.tolist() == np.asarray(
            m.__dict__["_deferred_source"]["face_attr"]).tolist()
        compare_mesh(store, m)               # this touch builds it
        compare_weld(store, m)
        n += 1
    assert n >= 3


def test_duplicate_and_shut_edges(binary, tmp_path):
    scene = Scene()
    m = scene.mesh
    m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    e = m.add_edge(V(5, 0, 0), V(6, 0, 0))
    e.soft, e.curve, e.layer = True, 3, "Guias"
    m.add_edge(V(6, 0, 0), V(6, 1, 0)).hidden = True
    shut = m.add_edge(V(7, 0, 0), V(7.5, 0, 0))
    shut.v1.position = V(7 + 2e-6, 0, 0)
    tri = m.add_face([V(9, 0, 0), V(10, 0, 0), V(10, 1, 0), V(9.5, 1, 0)])
    tri.loop[3].position = V(10 + 3e-6, 1, 0)
    m.add_edge(V(5, 0, 0), V(6, 0, 0))
    path = tmp_path / "doc.igz"
    igz.save_scene(scene, path)
    s2 = Scene()
    igz.load_into(s2, path)
    store = store_of_source(s2.mesh.__dict__["_deferred_source"])
    compare_plan(lazy_mesh.plan_of(s2.mesh), store)
    compare_mesh(store, s2.mesh)


@pytest.mark.parametrize("seed", range(12))
def test_crowded_blocks_against_bulk_mesh(seed):
    """Raw arrays, no file: the degenerate shapes a damaged or odd file
    can hold, against the bulk path and the plan (bulk and sequential)."""
    blk = crowded_block(random.Random(seed))
    pos, edges, flags, curves, layers, names, rings, sizes, counts = blk
    store = store_of_block(blk)
    m = build_mesh(*blk)
    compare_mesh(store, m)
    compare_weld(store, m)
    for sequential in (False, True):
        plan = lazy_mesh.plan(pos, edges, flags, curves, layers, names,
                              rings, sizes, counts, sequential=sequential)
        compare_plan(plan, store_of_block(blk, sequential))


def test_bad_blocks_are_refused():
    pos = np.zeros((3, 3), np.float32)
    e = np.zeros((0, 2), np.int64)
    ok = dict(rings=np.array([0, 1, 2]), sizes=np.array([3]),
              counts=np.array([1]))
    for bad in (dict(ok, rings=np.array([0, 1, 5])),
                dict(ok, sizes=np.array([2])),
                dict(ok, counts=np.array([2])),
                dict(ok, sizes=np.array([3, 0]), counts=np.array([2]))):
        with pytest.raises(ValueError):
            mod.MeshStore.from_block(_ns(), pos, e, None, None, None, [],
                                     bad["rings"], bad["sizes"],
                                     bad["counts"])
    with pytest.raises(ValueError):
        mod.MeshStore.from_block(_ns(), pos, np.array([[0, 3]]), None, None,
                                 None, [], *ok.values())


# ---- Keys ----------------------------------------------------------------

def test_keys_are_opaque_ints_in_the_mesh_namespace_space():
    blk = crowded_block(random.Random(3))
    store = store_of_block(blk)
    nv, ne, nf = store.counts()
    keys = set()
    for kind, n in (("vertex", nv), ("edge", ne), ("face", nf)):
        local = store.local_keys(kind)
        assert len(local) == n
        full = [store.key_base | int(k) for k in local.tolist()]
        assert full == [store.key_at(kind, i) for i in range(n)]
        assert [store.index_of(k) for k in full] == list(range(n))
        assert all(k >> 40 == store.namespace for k in full)
        keys.update(full)
    assert len(keys) == nv + ne + nf          # unique across kinds
    other = store.copy(_ns())
    k = store.key_at("vertex", 0)
    assert other.is_live(other.key_at("vertex", 0))
    assert not other.is_live(k)               # a copy's elements are new
    with pytest.raises(KeyError):
        other.move_vertex(k, (1, 0, 0))


# ---- move_vertex / place_vertex -----------------------------------------

def _random_ops(rng, n_vertices, n_ops):
    for _ in range(n_ops):
        i = rng.randrange(n_vertices)
        r = rng.random()
        if r < 0.35:
            yield "onto", i, rng.randrange(n_vertices)
        elif r < 0.7:
            yield "move", i, tuple(rng.choice([0, 4.9e-5, -5.1e-5, 1e-6, 0.5,
                                               -0.5, 1.0]) for _ in range(3))
        else:
            yield "place", i, tuple(rng.randrange(-4, 4) * 0.5 + rng.choice(
                [0, 4.9e-5, 5.1e-5, -1e-6]) for _ in range(3))


@pytest.mark.parametrize("seed", range(8))
def test_moves_rekey_like_the_mesh(seed, explicit_store_meshes):
    rng = random.Random(100 + seed)
    blk = crowded_block(rng, n_points=120, n_edges=90, n_faces=30)
    store = store_of_block(blk)
    m = build_mesh(*blk)
    compare_weld(store, m)
    keys = [store.key_at("vertex", i) for i in range(len(m.vertices))]
    rev = store.revision
    for op, i, arg in _random_ops(rng, len(m.vertices), 300):
        v = m.vertices[i]
        if op == "onto":
            # Moved exactly onto another vertex (a Move snapping to a
            # corner): its cell is held, the mover waits.
            d = m.vertices[arg].position - v.position
            rec = store.move_vertex(keys[i], d.toTuple())
            m.move_vertex(v, d)
        elif op == "move":
            rec = store.move_vertex(keys[i], arg)
            m.move_vertex(v, V(*arg))
        else:
            rec = store.place_vertex(keys[i], arg)
            m.place_vertex(v, V(*arg))
        assert rec.moved == [keys[i]]
        assert rec.revision == rev + 1
        rev = rec.revision
        compare_weld(store, m)
        # Welding a new point would pick the same vertex.
        probe = V(*arg) if op == "place" else v.position
        hit = m.vertex_at(probe)
        got = store.vertex_at(probe.toTuple())
        assert got == (None if hit is None else keys[m.vertices.index(hit)])
    # Keys never moved: the i-th vertex is still the i-th key.
    assert [store.key_at("vertex", i) for i in range(len(keys))] == keys


def test_moving_along_itself_promotes_the_waiter():
    """``Mesh._rekey``'s own case: two chained lines shifted one length
    down their axis — each vertex waits for the next one's cell."""
    pos = np.array([[0, 0, 0], [1, 0, 0], [2, 0, 0], [3, 3, 3]], np.float32)
    edges = np.array([[0, 1], [1, 2]], np.int64)
    z = np.zeros(2, np.int64)
    blk = (pos, edges, np.zeros(2, np.uint8), z - 1, z - 1, [],
           np.zeros(0, np.int64), np.zeros(0, np.int64), np.zeros(0, np.int64))
    store, m = store_of_block(blk), build_mesh(*blk)
    keys = [store.key_at("vertex", i) for i in range(4)]
    # The edgeless vertex parks on vertex 0's cell first: it waits there
    # and, having no edges, is never promoted.
    for i, d in ((3, (-3, -3, -3)), (0, (1, 0, 0)), (1, (1, 0, 0)),
                 (2, (1, 0, 0))):
        store.move_vertex(keys[i], d)
        m.move_vertex(m.vertices[i], V(*d))
        compare_weld(store, m)
    assert store.vertex_at((1, 0, 0)) == keys[0]


def test_faces_around_a_moved_vertex():
    blk = crowded_block(random.Random(4))
    store = store_of_block(blk)
    m = build_mesh(*blk)
    fkey = {f.key: store.key_at("face", i) for i, f in enumerate(m.faces)}
    for i, v in enumerate(m.vertices[:40]):
        want = {fkey[f.key] for f in v.faces()}
        got = store.faces_around([store.key_at("vertex", i)])
        assert len(got) == len(set(got))
        assert set(got) == want


# ---- Snapshots -----------------------------------------------------------

@pytest.mark.parametrize("seed", range(4))
def test_snapshot_restore_is_capture_restore(seed, explicit_store_meshes):
    rng = random.Random(200 + seed)
    blk = crowded_block(rng, n_points=120, n_edges=90, n_faces=30)
    store = store_of_block(blk)
    m = build_mesh(*blk)
    keys = [store.key_at("vertex", i) for i in range(len(m.vertices))]
    ops = list(_random_ops(rng, len(m.vertices), 60))

    def run(ops):
        for op, i, arg in ops:
            v = m.vertices[i]
            if op == "onto":
                d = m.vertices[arg].position - v.position
                store.move_vertex(keys[i], d.toTuple())
                m.move_vertex(v, d)
            elif op == "move":
                store.move_vertex(keys[i], arg)
                m.move_vertex(v, V(*arg))
            else:
                store.place_vertex(keys[i], arg)
                m.place_vertex(v, V(*arg))

    run(ops[:20])
    snap, state = store.snapshot(), m.capture_state()
    assert store.same_as(snap) and m.capture_state() == state
    run(ops[20:])
    assert store.same_as(snap) == (m.capture_state() == state)
    rev = store.revision
    assert store.restore(snap) == rev + 1
    m.restore_state(state)
    compare_weld(store, m)                    # _displaced cleared on both
    assert store.same_as(snap) and m.capture_state() == state
    assert [store.key_at("vertex", i) for i in range(len(keys))] == keys
    # The snapshot is frozen: editing after a restore leaves it intact,
    # and restoring it again gives the same state back.
    run(ops[20:40])
    store.restore(snap)
    m.restore_state(state)
    compare_weld(store, m)


def test_snapshot_shares_chunks_until_written():
    n = 60
    g = np.stack(np.meshgrid(np.arange(n), np.arange(n), np.arange(n),
                             indexing="ij"), -1).reshape(-1, 3) * 0.1
    pos = g.astype(np.float32)
    store = mod.MeshStore.from_block(
        _ns(), pos, np.zeros((0, 2), np.int64), None, None, None, [],
        np.zeros(0, np.int64), np.zeros(0, np.int64), np.zeros(0, np.int64),
        scale=SCALE, tol2=TOL2)
    snap = store.snapshot()
    shared, total = store.shared_chunks(snap)
    assert shared == total
    store.move_vertex(store.key_at("vertex", 1000), (0.05, 0, 0))
    shared, total = store.shared_chunks(snap)
    assert total - shared <= 3                # one position chunk, ≤2 shards
    assert not store.same_as(snap)
    assert snap.counts() == store.counts()


def test_foreign_snapshot_needs_a_fresh_namespace():
    """``Scene.share_back``: ``proto.restore_state(local.capture_state())``
    — the store adopts the other's elements under new keys."""
    a = store_of_block(crowded_block(random.Random(1)))
    b = store_of_block(crowded_block(random.Random(2)))
    snap = b.snapshot()
    with pytest.raises(ValueError):
        a.restore(snap)
    ns = _ns()
    a.restore(snap, namespace=ns)
    assert a.namespace == ns
    assert a.counts() == b.counts()
    assert a.arrays()["pos"].tolist() == b.arrays()["pos"].tolist()
    assert not a.is_live(b.key_at("vertex", 0))
    assert not a.same_as(snap)                # other lineage: not "unchanged"


def test_instance_frame_weld_snapshot_and_camera_origin():
    from ingetrazo_core import MeshStore
    origin = (1e12, -1e12, 6e11)
    matrix = (0, -2, 0, 0, -3, 0, 0, 0, 0, 0, 5, 0, *origin, 1)
    model = MeshStore(41)
    key, _ = model.vertex((0.000125, 1, 0))
    frame = model._in_instance_frame(matrix, origin)
    assert tuple(frame.vertex_position(key)) == (-3, -0.00025, 0)
    assert frame.vertex_at((-3, -0.00025, 0)) == key
    snap = frame.snapshot()
    frame.move_vertex(key, (0, -0.00025, 0))
    model._publish_frame(frame)
    assert tuple(model.vertex_position(key)) == (0.00025, 1, 0)
    model.restore(snap)
    assert tuple(model.vertex_position(key)) == (0.000125, 1, 0)
    assert model.vertex_at((0.000125, 1, 0)) == key
    model.check()
    with pytest.raises(ValueError):
        model._in_instance_frame([0.] * 16, origin)
