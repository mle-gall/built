"""Overlap healing returns the exact removed keys in the command's order."""
import pytest
from built_core import MeshStore
from core.scene import Scene
from core.topology import heal_overlapping_faces, winding_footprint
from tests.test_store_mesh import fingerprint


@pytest.mark.parametrize("subdivided", [False,True])
def test_heal_reports_original_keys(monkeypatch, subdivided):
    def make():
        mesh = Scene().mesh
        mesh.add_face([(0,0,0),(8,0,0),(8,8,0),(0,8,0)])
        if subdivided:
            for a,b in [(.2,3.8),(4.2,7.8)]:
                mesh.add_face([(a,.2,0),(b,.2,0),(b,7.8,0),(a,7.8,0)])
        return mesh
    mesh, ref = make(),make()
    keys = [f.key for f in mesh.faces]
    old = {f.key:i for i,f in enumerate(ref.faces)}
    removed = heal_overlapping_faces(ref, footprint=winding_footprint(ref))
    start = mesh._mut_serial
    faces,dirty = MeshStore.command_heal_overlaps(mesh)
    assert faces == [keys[old[f.key]] for f in removed]
    assert bool(faces) == subdivided
    assert dirty == mesh._mut_serial-start
    assert fingerprint(mesh) == fingerprint(ref)
