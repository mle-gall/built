"""Keyed AddFace results against the original command's ordered mesh edit."""
from types import SimpleNamespace

import pytest
from ingetrazo_core import Matrix4, MeshStore, Vec3

from core.history import AddFaceCommand, History, SnapshotMutation
from core.mesh import Mesh
from core.scene import Scene
from core import store_mesh
from tests.test_store_mesh import fingerprint


def rectangle(x, y, w, h):
    return [(x,y,0.), (x+w,y,0.), (x+w,y+h,0.), (x,y+h,0.)]


@pytest.mark.parametrize("scans", ["0", "1"])
@pytest.mark.parametrize("case", ["independent", "mother", "swap", "carve", "enclosed", "preset", "far"])
def test_keyed_add_face_matches_command(monkeypatch, case, scans):
    monkeypatch.setenv("INGETRAZO_SCANS", scans)
    mesh, reference = Mesh(), Mesh()
    offset = 6_860_000 if case == "far" else 0
    base = rectangle(offset, 0, 8, 8)
    outer = rectangle(offset+2, 2, 2, 2)
    mother_holes = None
    holes = None
    if case == "swap":
        mother_holes, outer = [rectangle(2, 2, 1, 1)], rectangle(3, 2, 1, 1)
    if case == "carve":
        outer = rectangle(2, 0, 2, 2)
    if case == "enclosed":
        base, outer = outer, base
    if case == "preset":
        outer = rectangle(12, 0, 8, 8)
        holes = [rectangle(13, 1, 5, 5), rectangle(14, 2, 1, 1)]
    props = {"mat": "paste", "custom": 13}
    for m in (mesh, reference):
        face = m.add_face(base, mother_holes)
        face.attrs.update({"mat":"wall", "texture":{"source":"wall"}, "extra":5})
    mother = mesh.faces[0]
    before = mesh._mut_serial
    divide = case != "independent"
    result = MeshStore.command_add_face(mesh, outer, holes, props, True, divide)
    command = AddFaceCommand(outer, auto=divide, holes=holes, attrs=props, interior=True)
    command.do(SimpleNamespace(mesh=reference, version=0))
    key, dirty, source, remainder, swapped = result
    assert dirty == mesh._mut_serial - before
    assert fingerprint(mesh) == fingerprint(reference)
    assert mesh._mut_serial == reference._mut_serial
    assert mesh.revision == reference.revision
    drawn = next(f for f in mesh.faces if f.key == key)
    assert swapped == bool(command._hole_swaps)
    if case in ("mother", "swap", "carve", "far"):
        assert source == mother.key
        assert drawn.attrs["mat"] == "paste"
        assert drawn.attrs["texture"] is not mother.attrs["texture"]
        assert (len(remainder) > 0) == (case == "carve")
        for replaced in remainder:
            assert next(f for f in mesh.faces if f.key == replaced).attrs == mother.attrs
    else:
        assert source is None and not remainder


def test_add_face_occurrence_undo_redo_reuses_key_and_paint(monkeypatch):
    scene = Scene()
    mesh = scene.mesh
    frame = Matrix4()
    frame.translate(20, 30, 0)
    occurrence = store_mesh.instance_view(mesh, frame.data())
    history = History(scene)
    results = []
    def draw(_):
        results.append(MeshStore.command_add_face(occurrence, rectangle(20, 30, 3, 2),
                                                  attrs={"mat":"bronze"}))
    history.execute(SnapshotMutation(draw, mesh=occurrence))
    key, dirty, mother, pieces, swapped = results[0]
    assert (dirty, mother, pieces, swapped) == (5, None, [], False)
    assert mesh.faces[0].vertices[0] == Vec3()
    assert mesh.faces[0].attrs["mat"] == "bronze"
    assert history.undo() and not mesh.faces
    assert history.redo() and mesh.faces[0].key == occurrence.faces[0].key == key
    assert occurrence.faces[0].attrs["mat"] == "bronze"


def test_face_uses_planned_binary64_corner():
    mesh = Mesh()
    planned = (1.000000000000001, 0., 0.)
    mesh._planned_points = {(1., 0., 0.): planned}
    MeshStore.command_add_face(mesh, rectangle(0, 0, 1, 1))
    assert planned in [tuple(v.position_tuple()) for v in mesh.vertices]


@pytest.mark.parametrize("bad", [float("nan"), float("inf")])
def test_plain_face_refusal_keeps_allocator_and_revision(monkeypatch, bad):
    mesh = Mesh()
    before = (mesh._store.revision, mesh._mut_serial)
    with pytest.raises((ValueError, OverflowError)):
        MeshStore.command_add_face(mesh, [(0., 0., 0.), (1., 0., 0.), (bad, 0., 0.)],
                                   divide=False)
    assert not mesh.vertices and not mesh.edges and not mesh.faces
    assert (mesh._store.revision, mesh._mut_serial) == before
    key, dirty, *_ = MeshStore.command_add_face(mesh, rectangle(0, 0, 1, 1), divide=False)
    other = Mesh()
    expected, *_ = MeshStore.command_add_face(other, rectangle(0, 0, 1, 1), divide=False)
    assert (key & ((1 << 40) - 1), dirty) == (expected & ((1 << 40) - 1), 5)
