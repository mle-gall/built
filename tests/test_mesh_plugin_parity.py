"""Mesh plugin observables recorded before retiring the Python facade."""
import copy
import json
import pickle
from pathlib import Path

from core.mesh import Edge, Face, Mesh, Vertex
from ingetrazo_core import Vec3


class PluginMesh(Mesh):
    pass


FIXTURES = Path(__file__).with_name("fixtures")


def sample():
    mesh = PluginMesh()
    mesh.plugin_label = "wall"
    mesh.add_face([Vec3(0, 0, 0), Vec3(3, 0, 0), Vec3(3, 2, 0), Vec3(0, 2, 0)])
    mesh.faces[0].attrs["texture"] = {"path": "wall.png"}
    mesh.edges[0].soft = True
    return mesh


def describe(mesh):
    verts = list(mesh.vertices)
    return {
        "type": type(mesh).__name__, "label": mesh.plugin_label,
        "positions": [list(v.position_tuple()) for v in verts],
        "edges": [[verts.index(e.v0), verts.index(e.v1), e.soft] for e in mesh.edges],
        "faces": [[verts.index(v) for v in f.loop] for f in mesh.faces],
        "area": mesh.faces[0].net_area(), "attrs": dict(mesh.faces[0].attrs),
        "interfaces": [isinstance(mesh.vertices[0], Vertex), isinstance(mesh.edges[0], Edge),
                       isinstance(mesh.faces[0], Face)],
    }


def test_mesh_plugin_parity():
    mesh = sample()
    expected = json.loads((FIXTURES / "mesh_plugin_python.json").read_text())
    snap = mesh.capture_state()
    mesh.move_vertex(mesh.vertices[0], Vec3(1, 0, 0))
    mesh.restore_state(snap)
    for owner, face in [(mesh, mesh.faces[0]), copy.deepcopy((mesh, mesh.faces[0])),
                        pickle.loads(pickle.dumps((mesh, mesh.faces[0]))),
                        pickle.loads((FIXTURES / "mesh_plugin_python.pickle").read_bytes())]:
        assert describe(owner) == expected
        assert face._m is owner and face == owner.faces[0]
