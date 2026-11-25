# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The store-backed ``Mesh`` (``core/store_mesh.py``, port step 1) against
the Python mesh it replaces on the snapshot, construction and lazy-load
paths: the same mesh read through views, writes that stay in the store,
materialise-on-write with keys kept, store snapshots through history, bulk
construction, copies and share-back."""
from __future__ import annotations

import built_core
import copy
import gzip
import json
from pathlib import Path
import random

import numpy as np
import pytest
from built_core import Vec3 as V

import core._native as _native
from built_core import mesh as M
from built_core import store_mesh as S
from built_core.history import (AddEdgeCommand, DeleteEdgesCommand, History,
                          MoveVerticesCommand)
from built_core.scene import Scene
from formats import igz
from tests.test_deferred_mesh import _meshes, _random_scene

pytestmark = pytest.mark.skipif(
    _native.available() is None
    or not hasattr(_native.available(), "MeshStore"),
    reason="native module without MeshStore")


@pytest.fixture
def binary(monkeypatch):
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    monkeypatch.setattr(igz, "_BULK_MIN", 1)
    monkeypatch.setattr(_native, "wanted", lambda: True)


def fingerprint(m) -> dict:
    """Everything observable, by list index (keys differ between engines)."""
    vid = {v.key: i for i, v in enumerate(m.vertices)}
    fid = {f.key: i for i, f in enumerate(m.faces)}
    return {
        "v": [v.position.toTuple() for v in m.vertices],
        "vdeg": [len(v.edges) for v in m.vertices],
        "e": [(vid[e.v0.key], vid[e.v1.key], bool(e.soft), bool(e.hidden),
               e.curve, e.layer, [fid[f.key] for f in e.faces])
              for e in m.edges],
        "f": [([vid[v.key] for v in f.loop],
               [[vid[v.key] for v in h] for h in f.hole_loops],
               sorted(f.attrs.items(), key=str), bool(f.interior),
               f.normal().toTuple(), f.area())
              for f in m.faces],
    }


def _open(path):

    scene = Scene()
    igz.load_into(scene, path)
    for m in _meshes(scene):
        if "_deferred" in m.__dict__:
            m.__dict__["_deferred_geom"] = object()
        len(m.faces)                              # first touch
        assert "_deferred_geom" not in m.__dict__
    return scene


@pytest.mark.parametrize("seed", range(4))
def test_first_touch_is_a_store_reading_like_the_built_mesh(
        binary, monkeypatch, tmp_path, seed):
    path = tmp_path / "doc.igz"
    igz.save_scene(_random_scene(random.Random(seed)), path)
    scene = _open(path)
    meshes = list(_meshes(scene))
    with gzip.open(Path(__file__).parent / "fixtures" / "deferred_first_touch.json.gz", "rt") as src:
        expected = json.load(src)[seed]
    assert len(meshes) == len(expected)
    for mesh, state in zip(meshes, expected):
        assert json.loads(json.dumps(fingerprint(mesh))) == state
        assert type(mesh) is S.StoreMesh


def _store_mesh(tmp_path, seed=3):

    path = tmp_path / "one.igz"
    scene = _random_scene(random.Random(seed))
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    m = back.mesh
    len(m.faces)
    assert type(m) is S.StoreMesh
    return back, m


def test_views_are_elements_compared_by_key(binary, monkeypatch, tmp_path):
    _scene, m = _store_mesh(tmp_path)
    f1, f2 = m.faces[0], m.faces[0]
    assert f1 is not f2 and f1 == f2 and hash(f1) == hash(f2)
    assert isinstance(f1, M.Face) and isinstance(m.edges[0], M.Edge)
    assert f1 in m.faces and f1 in set(m.faces)
    assert m.faces.index(f1) == 0
    e = f1.loop and m.find_edge(f1.loop[0], f1.loop[1])
    assert e is not None and f1 in e.faces
    assert e.other(e.v0) == e.v1 and e.has(e.v1)
    assert m.vertex_at(f1.loop[0].position) == f1.loop[0]
    assert m.registers(f1.loop[0])
    assert M.element_identity(f1) == f1.key


def test_flag_and_attr_writes_stay_in_the_store(binary, monkeypatch,
                                                tmp_path):
    _scene, m = _store_mesh(tmp_path)
    e, f = m.edges[0], m.faces[0]
    e.soft, e.hidden, e.curve, e.layer = True, True, 77, "Tag"
    f.interior = True
    f.attrs["color"] = [1.0, 0.0, 0.0]
    f.attrs.setdefault("texture", {})["rot"] = 0.5   # nested, in place
    assert type(m) is S.StoreMesh
    e2, f2 = m.edges[0], m.faces[0]
    assert (e2.soft, e2.hidden, e2.curve, e2.layer) == (True, True, 77, "Tag")
    assert f2.interior and f2.attrs["color"] == [1.0, 0.0, 0.0]
    assert f2.attrs["texture"] == {"rot": 0.5}
    m.move_vertex(m.vertices[0], V(0.25, 0, 0))
    assert type(m) is S.StoreMesh
    # An edge drawn stays in the store too (port step 2).
    m.add_edge(V(50, 0, 0), V(51, 0, 0))
    assert type(m) is S.StoreMesh
    # Materialised, every write is still there, under the same keys.
    keys = [x.key for x in m.faces]
    assert type(m) is S.StoreMesh
    assert [x.key for x in m.faces][:len(keys)] == keys
    ee = m.edges[0]
    assert (ee.soft, ee.hidden, ee.curve, ee.layer) == (True, True, 77, "Tag")
    assert m.faces[0].attrs["texture"] == {"rot": 0.5} and m.faces[0].interior


def test_removal_and_relink_preserve_held_views(
        binary, monkeypatch, tmp_path):
    _scene, m = _store_mesh(tmp_path)
    face = m.faces[2]
    loop = face.loop
    edge = m.find_edge(loop[0], loop[1])
    m.remove_face(face)
    assert type(m) is S.StoreMesh
    assert face not in m.faces and face.key not in {f.key for f in m.faces}
    assert all(f.key != face.key for f in edge.faces)
    m.relink_face(face)                          # the view resolves
    assert face in m.faces
    obj = m.faces[-1]
    assert obj.key == face.key and obj == face and face == obj
    assert {face} == {obj}                       # same hash both ways
    # A held view's in-place write edits the mesh.
    v = m.faces[0]
    n = len(v.loop)
    v.loop.reverse()
    assert [x.key for x in m.faces[0].loop][0] == [x.key for x in v.loop][0]
    assert len(m.faces[0].loop) == n


def test_snapshot_restore_preserves_held_views(
        binary, monkeypatch, tmp_path):
    _scene, m = _store_mesh(tmp_path)
    before = fingerprint(m)
    s0 = m.capture_state()
    assert isinstance(s0, S.StoreState) and m.capture_state() == s0
    m.add_face([V(40, 0, 0), V(41, 0, 0), V(41, 1, 0)])
    assert type(m) is S.StoreMesh
    assert m.capture_state() != s0
    held = m.faces[0]
    m.restore_state(s0)
    assert type(m) is S.StoreMesh and m.capture_state() == s0
    assert fingerprint(m) == before
    # The next materialisation hands back the very objects.
    assert m.faces[0] == held
    # A Python snapshot of the materialised mesh equals the store snapshot
    # it came from, by content.
    m.restore_state(s0)
    m._registry                                  # materialise, no edit
    assert m.capture_state() == s0 and s0 == m.capture_state()


def test_history_on_a_store_mesh(binary, monkeypatch, tmp_path):
    path = tmp_path / "history.igz"
    igz.save_scene(_random_scene(random.Random(7)), path)
    scene = _open(path)
    h = History(scene)
    start = V(scene.mesh.edges[0].a)
    states = [fingerprint(scene.mesh)]
    for command in (AddEdgeCommand(V(70, 0, 0), V(71, 0, 0)),
                    DeleteEdgesCommand([scene.mesh.edges[1]]),
                    MoveVerticesCommand([start], V(0, 0, 0.5))):
        h.execute(command)
        assert h.last_error is None
        states.append(fingerprint(scene.mesh))
    for prior in reversed(states[:-1]):
        assert h.undo() and fingerprint(scene.mesh) == prior
    for next_state in states[1:]:
        assert h.redo() and fingerprint(scene.mesh) == next_state


def test_a_move_command_never_builds_python_objects(binary, monkeypatch,
                                                     tmp_path):
    scene, m = _store_mesh(tmp_path, seed=1)
    h = History(scene)
    corner = V(m.faces[0].loop[0].position)
    h.execute(MoveVerticesCommand([corner], V(0, 0, 0.001)))
    h.undo()
    h.redo()
    assert h.last_error is None


@pytest.mark.parametrize("seed", range(3))
def test_bulk_construction_into_the_store(seed):
    rng = random.Random(seed)
    faces = []
    for _ in range(60):
        o = [rng.randrange(-5, 5) * 0.5 for _ in range(3)]
        faces.append([[o[0] + dx, o[1] + dy, o[2]] for dx, dy in
                      ((0, 0), (1, 0), (1, 1), (0, 1))])
    flat = np.array([p for f in faces for p in f], dtype=np.float64)
    attrs = [({"color": [0.5, 0.5, 0.5]} if i % 3 else None)
             for i in range(len(faces))]
    epts = np.array([[0, 0, 0], [3, 0, 0], [3, 0, 0], [3, 3, 0]], float)
    m = M.Mesh()
    vobjs, inv = m.bulk_weld(np.concatenate([epts, flat]))
    emap = m.add_edges_welded(vobjs, inv[0:4:2], inv[1:4:2],
                              [(True, 5, "T"), (False, None, None, True)])
    u = len(vobjs)
    a, b = int(inv[0]), int(inv[1])
    e = emap.get(a * u + b if a <= b else b * u + a)
    e.layer = "U"                    # stamped between edges and faces
    m.add_faces_welded(vobjs, inv[4:], np.full(len(faces), 4),
                       np.ones(len(faces), dtype=int), attrs,
                       edge_map=emap)
    with gzip.open(Path(__file__).parent / "fixtures" / "bulk_mesh_construction.json.gz", "rt") as src:
        expected = json.load(src)[seed]
    assert json.loads(json.dumps(fingerprint(m))) == expected
    assert type(m) is S.StoreMesh


def test_copies_and_share_back(binary, monkeypatch, tmp_path):
    _scene, m = _store_mesh(tmp_path)
    c = copy.deepcopy(m)
    assert type(c) is S.StoreMesh and fingerprint(c) == fingerprint(m)
    assert not {f.key for f in c.faces} & {f.key for f in m.faces}
    m.add_edge(V(80, 0, 0), V(81, 0, 0))          # stays in the store
    d = copy.deepcopy(m)
    assert fingerprint(d) == fingerprint(m)
    # Share-back: a plain Python mesh adopts a store snapshot, new keys.
    proto = M.Mesh()
    proto.add_edge(V(0, 0, 0), V(1, 0, 0))
    proto.restore_state(c.capture_state())
    assert type(proto) is S.StoreMesh
    assert fingerprint(proto) == fingerprint(c)
    assert not {f.key for f in proto.faces} & {f.key for f in c.faces}


def test_link_hooks_relabel_like_the_built_mesh(binary, monkeypatch,
                                                tmp_path):
    path = tmp_path / "doc.igz"
    sc = _random_scene(random.Random(4))
    for f in sc.mesh.faces[::2]:
        f.attrs["layer"] = "A"
    for e in sc.mesh.edges[::3]:
        e.layer = "B"
    igz.save_scene(sc, path)
    before = _open(path).mesh
    scene = Scene()
    igz.load_into(scene, path)
    mesh = scene.mesh
    assert type(mesh) is M.DeferredMesh
    renames = {"A": "A2", "B": "B2"}

    def rename(name):
        return renames.get(name, name)
    built_core.link_relabel([], [mesh], renames)
    assert type(mesh) is M.DeferredMesh                     # relabelled unbuilt
    from built_core.layers import layer_of
    for actual, original in zip((*mesh.faces, *mesh.edges), (*before.faces, *before.edges), strict=True):
        assert layer_of(actual) == rename(layer_of(original))


@pytest.mark.parametrize("seed", range(3))
def test_the_stores_warped_faces_are_is_planars(monkeypatch, seed):
    """``MeshStore.nonplanar_faces`` is ``topology.is_planar`` over every
    face with more than three corners (holes included), bit for bit."""
    from built_core.topology import _PLANAR_TOLERANCE, is_planar
    rng = random.Random(seed)
    flat, sizes, counts = [], [], []
    for _ in range(300):
        o = [rng.uniform(-50, 50) for _ in range(3)]
        n = rng.choice([3, 4, 5, 6])
        ring = [[o[0] + rng.uniform(0, 2), o[1] + rng.uniform(0, 2),
                 o[2] + rng.choice([0, 0, 0, 5e-4, 1e-3, 1.5e-3, 0.3])
                 * rng.random()] for _ in range(n)]
        flat += ring
        sizes.append(n)
        rings = 1
        if rng.random() < 0.2:
            hole = [[o[0] + 0.5 + 0.1 * k, o[1] + 0.5, o[2]] for k in range(3)]
            flat += hole
            sizes.append(3)
            rings = 2
        counts.append(rings)

    m = M.Mesh()
    m.add_faces_bulk(np.array(flat), sizes, counts)
    assert type(m) is S.StoreMesh
    want = [f.key for f in m.faces
            if len(f.vertices) + sum(len(h) for h in f.holes) > 3
            and not is_planar(list(f.vertices) + [p for h in f.holes
                                                   for p in h],
                              _PLANAR_TOLERANCE)]
    got = [f.key for f in m.nonplanar_faces(_PLANAR_TOLERANCE)]
    assert got == want and want


def test_cell_queries_are_the_python_scans(binary, monkeypatch, tmp_path):
    from built_core.topology import _key
    _scene, m = _store_mesh(tmp_path)
    vs = list(m.vertices)
    for cells in ({_key(vs[0].position)},
                  {_key(v.position) for v in vs[::3]},
                  {_key(v.position + V(0, 0, 4.9e-5)) for v in vs[::2]}):
        assert [v.key for v in m.vertices_in_cells(cells, _key)] == [
            v.key for v in vs if _key(v.position) in cells]
    keys = {v.key for v in vs[::5]}
    assert m.cells_of(keys, _key) == {_key(v.position) for v in vs
                                  if v.key in keys}
    m.faces[1].attrs["texture"] = {"path": "x.png"}
    assert [f.key for f in m.textured_faces()] == [m.faces[1].key]
    assert type(m) is S.StoreMesh

class PluginMesh(M.Mesh):
    pass


def test_plugin_mesh_subclass_roundtrips_with_native_elements():
    import pickle
    from built_core import MeshElements

    mesh = PluginMesh()
    mesh.plugin_label = "assembly"
    mesh.add_edge(V(0, 0, 0), V(1, 0, 0))
    for restored in (copy.deepcopy(mesh), pickle.loads(pickle.dumps(mesh))):
        assert type(restored) is PluginMesh
        assert restored.plugin_label == "assembly"
        assert isinstance(restored.edges, MeshElements)
        assert restored.edges[0].v1.position_tuple() == (1., 0., 0.)
        restored.edges[0].hidden = True
        assert not mesh.edges[0].hidden
