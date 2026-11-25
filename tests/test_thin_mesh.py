# SPDX-License-Identifier: GPL-3.0-or-later
from copy import deepcopy

import pytest
from ingetrazo_core import Vec3 as V

from core import _native
from core.group import Group
from core.history import History, SetFaceColorCommand, SnapshotMutation
from core.intersect import WITH_CONTEXT, build_intersect_faces
from core.mesh import Mesh
from core.protocol.commands import run
from core.scene import Scene
from core.store_mesh import StoreMesh
from tests.test_offset_regions import _VP, _offset
from tests.test_store_mesh import fingerprint
from tests.test_store_pieces_explode import _cube
from tools.followme import FollowMeTool


@pytest.fixture
def resident(monkeypatch):
    module = pytest.importorskip("ingetrazo_core")
    monkeypatch.setattr(_native, "native", module)
    monkeypatch.setattr(StoreMesh, "_materialise", lambda *_: pytest.fail("materialised"))


def test_store_tool_sequence(resident):
    scene = Scene()
    vp = _VP(scene)
    vp.set_hover = lambda entity: None
    history = vp.history
    mesh = scene.mesh
    assert type(mesh) is StoreMesh

    def command(name, **args):
        result = run(history, name, **args)
        assert result["ok"], result

    ring = [(0, 0, 0), (2, 0, 0), (2, 2, 0), (0, 2, 0)]
    command("add_edges", segments=list(zip(ring, ring[1:] + ring[:1])))
    command("push_pull", face=mesh.faces[0].key, distance=2, keep_base=True)
    command("move_vertices", vertices=mesh._store.keys("vertex"), delta=[1, 0, 0])
    top = max(mesh.faces, key=lambda f: f.centroid().z())
    revision = history.revision
    _offset(vp, top, .2)
    assert history.revision == revision + 1
    edge = min(mesh.edges, key=lambda e: e.a.z() + e.b.z())
    history.execute(SnapshotMutation(lambda _: mesh.fillet_edges([edge.key], .1)))
    assert history.revision == revision + 2, history.last_error

    path = [V(10, 0, 0), V(10, 2, 0), V(12, 2, 0)]
    face = mesh.add_face([path[0] + V(x, 0, z) for x, z in
                          [(0, 0), (1, 0), (1, 1), (0, 1)]])
    FollowMeTool()._commit(vp, face, path, False)
    assert history.revision == revision + 3, history.last_error
    plate = mesh.add_face([V(x, y, 1) for x, y in [(0,-1),(4,-1),(4,3),(0,3)]])
    scene.selection = {plate}
    count, edit = build_intersect_faces(scene, WITH_CONTEXT, [plate.key])
    assert count
    history.execute(edit)
    assert history.revision == revision + 4, history.last_error

    group = Group()
    _cube(group.mesh, x0=20)
    _cube(group.mesh, x0=24)
    scene.groups.append(group)
    command("split_into_pieces", placement=group.uid)
    assert type(group.mesh) is StoreMesh and len(group.children) == 2
    command("explode_group", placement=group.uid)
    command("explode_group", placement=scene.groups[0].uid)
    face = mesh.faces[-1]
    history.execute(SetFaceColorCommand([face], [1, 0, 0]))
    assert face.attrs["color"] == [1, 0, 0]
    command("undo")
    assert "color" not in face.attrs
    command("redo")
    assert face.attrs["color"] == [1, 0, 0]
    mesh._store.check()


def test_bulk_reverse_feed_and_ownership(resident, monkeypatch):
    scene = Scene()
    mesh = scene.mesh
    history = History(scene)
    faces = _cube(mesh)
    face = mesh.add_face([V(x,y,3) for x,y in [(0,0),(4,0),(4,4),(0,4)]],
                         [[V(x,y,3) for x,y in [(1,1),(1,2),(2,2),(2,1)]]])
    faces.append(face)
    keys = [f.key for f in faces]
    inputs = [(f.vertices, f.holes) for f in faces]
    before = fingerprint(mesh)
    with pytest.raises(KeyError):
        mesh._store.reverse_faces([keys[0], mesh.edges[0].key])
    assert fingerprint(mesh) == before
    assert not any(mesh._store.reverse_faces([keys[0], keys[0]]).modified.values())
    for phase in ("reverse_faces", "undo", "redo"):
        result = run(history, phase, **({"faces": keys} if phase == "reverse_faces" else {}))
        change, = result["step"].meshes
        assert set(change.modified["face"]) == set(keys)
        assert not any(change.created.values()) and not any(change.removed.values())
    after = fingerprint(mesh)
    for face, (outer, holes) in zip(mesh.faces, inputs, strict=True):
        assert list(face.vertices) == list(reversed(outer))
        assert list(face.holes) == [list(reversed(ring)) for ring in holes]
    clone = deepcopy(mesh)
    assert type(clone) is StoreMesh and fingerprint(clone) == after
    assert set(keys).isdisjoint(clone._store.keys("face"))
    snap = mesh.capture_state()
    mesh.clear()
    assert type(mesh) is StoreMesh and mesh._store.counts() == (0, 0, 0)
    mesh.restore_state(snap)
    assert mesh._store.keys("face") == keys and fingerprint(mesh) == after


def test_incremental_welded_import_and_snapshot_keys(resident, monkeypatch):
    def build(mesh):
        mesh.vertex(V(0, 0, 0))
        vertices, ids = mesh.bulk_weld([(0, 0, 0), (1, 0, 0), (0, 1, 0)])
        edges = mesh.add_edges_welded(vertices, ids[:1], ids[1:2], [(True, 3, "rim")])
        mesh.add_faces_welded(vertices, ids, [3], [1], [{"mat": "steel"}], edges)
        assert len(edges) == 3
        return fingerprint(mesh)

    mesh = Mesh()
    snap = mesh.capture_state()
    result = build(mesh)
    assert build(Mesh()) == result
    mesh.restore_state(snap)
    mesh.add_faces_bulk([(2,0,0), (3,0,0), (2,1,0)], [3], [1])
    filled = mesh.capture_state()
    keys = mesh._store.keys("face")
    mesh.restore_state(snap)
    mesh.restore_state(filled)
    assert mesh._store.keys("face") == keys
    clone, face = deepcopy((mesh, mesh.faces[0]))
    assert face == clone.faces[0] and face.key != keys[0]


def test_bulk_stages_share_lineage_and_preserve_discarded_key_generations(resident):
    mesh = Mesh()
    owner = mesh._store
    vertices, ids = mesh.bulk_weld([(0, 0, 0), (1, 0, 0), (0, 1, 0)])
    points = mesh.capture_state()
    edges = mesh.add_edges_welded(vertices, ids[:1], ids[1:2])
    (edge,) = mesh.edges
    mesh.add_faces_welded(vertices, ids, [3], [1], edge_map=edges)
    assert mesh._store is owner
    mesh.restore_state(points)
    new_edge = mesh.add_edge(V(1, 0, 0), V(0, 1, 0))
    assert new_edge.key != edge.key


def test_group_and_loose_reverse_report_both_owners(resident):
    from core.history import FlipFacesCommand

    scene = Scene()
    group = Group()
    scene.groups.append(group)
    faces = [_cube(owner)[0] for owner in (scene.mesh, group.mesh)]
    history = History(scene)
    history.execute(FlipFacesCommand(faces))
    for step in (history.feed.last, run(history, "undo")["step"]):
        assert {change.mesh: change.modified["face"] for change in step.meshes} == {
            scene.mesh: [faces[0].key], group.mesh: [faces[1].key]}


def test_compatibility_write_logs_and_releases_store(monkeypatch):
    import io
    from core import history as history_module
    from core.store_mesh import MaterialisedMesh

    monkeypatch.setattr(_native, "native", pytest.importorskip("ingetrazo_core"))
    monkeypatch.setenv("INGETRAZO_PERF", "1")
    output = io.StringIO()
    monkeypatch.setattr(history_module, "_PERF", True)
    monkeypatch.setattr(history_module, "_perf_file", output)
    mesh = Mesh()
    edge = mesh.add_edge(V(), V(1, 0, 0))
    snapshot = mesh.capture_state()
    edge.layer = {"custom": 1}
    assert type(mesh) is MaterialisedMesh and "_store" not in mesh.__dict__
    assert edge.layer == {"custom": 1}
    assert "_set_flag" in output.getvalue() and "mesh.materialise" in output.getvalue()
    mesh.clear()
    assert type(mesh) is StoreMesh and not mesh.edges
    mesh.restore_state(snapshot)
    assert type(mesh) is StoreMesh and edge.layer is None


def test_paint_record_keeps_faces_on_their_own_mesh(resident):
    scene = Scene()
    group = Group()
    scene.groups.append(group)
    faces = [_cube(owner)[0] for owner in (scene.mesh, group.mesh)]
    history = History(scene)
    history.execute(SetFaceColorCommand(faces, (0.1, 0.2, 0.3)))
    for step in (history.feed.last, run(history, "undo")["step"]):
        assert {change.mesh: change.modified["face"] for change in step.meshes} == {
            scene.mesh: [faces[0].key], group.mesh: [faces[1].key]}
