"""Grouped geometry returns source/copy keys in addition to the rebuilt mesh."""
import pytest
from ingetrazo_core import MeshStore
from core.history import ExplodeGroupCommand, History, SnapshotMutation
from tests.test_store_mesh import fingerprint
from tests.test_store_pieces_explode import _explode_scene, _translate


@pytest.mark.parametrize("placement", ["shared_wall", "separate", "coincident"])
def test_explode_result_matches_history(monkeypatch, placement):
    monkeypatch.setenv("INGETRAZO_STORE", "1")
    actual, group, _ = _explode_scene(True)
    expected, ref, _ = _explode_scene(True)
    if placement != "shared_wall":
        transform = _translate(10., 3., 1.) if placement == "separate" else None
        group.xform = ref.xform = transform
    transform = group.xform.data() if group.xform is not None else None
    mesh = actual.mesh
    start = mesh._mut_serial
    added, copies, planes, dirty = MeshStore.command_explode_geometry(
        mesh, group.mesh, transform, group.material)
    command = ExplodeGroupCommand(ref)
    command.do(expected)
    assert dirty == mesh._mut_serial - start
    assert mesh.revision == expected.mesh.revision
    assert fingerprint(mesh) == fingerprint(expected.mesh)
    assert len(added) == len(group.mesh.faces)
    assert planes == {"shared_wall": 1, "separate": 0, "coincident": 6}[placement]
    for kind in ("vertex", "edge", "face"):
        assert len(copies[kind]) == len(command.copies[0][2][kind])
        assert all(group.mesh._store.is_live(src) and mesh._store.is_live(dst)
                   for src, dst in copies[kind])


def test_explode_copies_survive_transaction_replay(monkeypatch):
    monkeypatch.setenv("INGETRAZO_STORE", "1")
    scene, group, _ = _explode_scene(True)
    mesh = scene.mesh
    history = History(scene)
    out = []
    history.execute(SnapshotMutation(lambda _: out.append(MeshStore.command_explode_geometry(
        mesh, group.mesh, group.xform.data(), group.material)), mesh=mesh))
    keys = mesh._store.keys("face")
    expected = dict(out[0][1])
    assert history.undo() and history.redo()
    assert mesh._store.keys("face") == keys
    assert all(mesh._store.is_live(k) for pairs in expected.values() for _,k in pairs)
