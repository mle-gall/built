"""The keyed rebuilds preserve geometry, winding, paint, dirty counts and selection ownership."""
import pytest

from ingetrazo_core import MeshStore, Vec3
from core.history import RebuildPlaneFacesCommand
from core.scene import Scene
from tests.test_store_mesh import fingerprint


def drawing():
    scene = Scene()
    mesh = scene.mesh
    a = mesh.add_face([(0,0,0), (4,0,0), (4,2,0), (0,2,0)])
    a.attrs["mat"] = "brick"
    b = mesh.add_face([(2,1,0), (5,1,0), (5,3,0), (2,3,0)])
    b.attrs["texture"] = {"sw": 2., "sh": 4.}
    mesh.add_face([(8,0,0), (8,2,0), (8,2,2)])
    mesh.edges[0].curve = 17
    mesh.edges[0].layer = "contour"
    return scene


@pytest.mark.parametrize("scope", ["whole", "two", "stale", "empty", "no_plane"])
def test_plane_result_matches_history(monkeypatch, scope):
    monkeypatch.setenv("INGETRAZO_STORE", "1")
    actual, expected = drawing(), drawing()
    mesh = actual.mesh
    origin = (0., 0., 12. if scope == "no_plane" else 0.)
    normal = (0., 0., 1.)
    keys = ref = None
    if scope in ("two", "stale", "empty"):
        keys = [f.key for f in mesh.faces[:2]]
        ref = list(expected.mesh.faces[:2])
        if scope == "empty":
            keys, ref = [], []
        elif scope == "stale":
            mesh.remove_face(mesh.faces[1])
            expected.mesh.remove_face(ref[1])
    start = mesh._mut_serial
    result = MeshStore.command_rebuild_plane(mesh, origin, normal, keys)
    command = RebuildPlaneFacesCommand(Vec3(*origin), Vec3(*normal), faces=ref)
    version = expected.version
    command.do(expected)
    assert result == (expected.version != version, command.rebuilt, mesh._mut_serial - start)
    assert mesh._mut_serial == expected.mesh._mut_serial
    assert fingerprint(mesh) == fingerprint(expected.mesh)


def test_plane_refusal_leaves_rows_untouched(monkeypatch):
    monkeypatch.setenv("INGETRAZO_STORE", "1")
    mesh = drawing().mesh
    before = fingerprint(mesh), mesh.revision
    mesh._planned_points = {}
    assert MeshStore.command_rebuild_plane(mesh, (0.,0.,0.), (0.,0.,1.)) is None
    assert (fingerprint(mesh), mesh.revision) == before
