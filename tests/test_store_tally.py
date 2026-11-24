"""The purge survey's tallies of a store mesh (Rust: store/tally.rs) agree
with reading every face and edge one by one."""
from collections import Counter

from ingetrazo_core import Vec3

from core.mesh import Mesh
from core.store_parts import layer_tally, material_names


def _mesh():
    mesh = Mesh()
    faces = [mesh.add_face([Vec3(x, 0, 0), Vec3(x + 1, 0, 0), Vec3(x + 1, 1, 0), Vec3(x, 1, 0)])
             for x in range(0, 12, 2)]
    faces[0].attrs["layer"] = "Walls"
    faces[1].attrs["layer"] = "Walls"
    faces[2].attrs["layer"] = "Slabs"
    faces[2].attrs["mat"] = "Oak"
    faces[3].attrs["mat"] = "Steel"
    faces[4].attrs["mat"] = ""
    edges = list(mesh.edges)
    edges[0].layer = "Lines"
    edges[3].layer = "Lines"
    edges[5].layer = "Walls"
    return mesh


def test_the_tag_tally_counts_every_face_and_edge():
    mesh = _mesh()
    got = Counter()
    assert layer_tally(mesh, got) is True
    want = Counter((f.attrs.get("layer") or None) for f in mesh.faces)
    want.update((getattr(e, "layer", None) or None) for e in mesh.edges)
    assert got == want


def test_material_names_leave_out_empties():
    mesh = _mesh()
    want = {f.attrs.get("mat") for f in mesh.faces} - {None, ""}
    assert material_names(mesh) == want == {"Oak", "Steel"}


def test_a_mesh_without_a_store_is_not_counted():
    class Plain:
        pass
    assert layer_tally(Plain(), Counter()) is False
    assert material_names(Plain()) is None
