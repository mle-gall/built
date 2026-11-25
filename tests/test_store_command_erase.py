"""The native erase stages bracket the history command's plane rebuilds."""
from types import SimpleNamespace

import pytest
from built_core import MeshStore
from core.history import EraseSelectionCommand
from core.mesh import Mesh
from tests.test_store_mesh import fingerprint


def patch(kind):
    m = Mesh()
    if kind == "orphan":
        m.add_edge((0,0,0),(1,0,0))
    elif kind == "seam":
        face = m.add_face([(0,0,0),(2,0,0),(2,2,0),(0,2,0)])
        m.edges[0].soft = True
        face.attrs["mat"] = "seam test"
    else:
        a = m.add_face([(0,0,0),(2,0,0),(2,1,0),(0,1,0)])
        b = m.add_face([(0,1,0),(2,1,0),(2,3,0),(0,3,0)])
        a.attrs.update({"mat":"small", "other": 1})
        b.attrs.update({"mat":"large", "other": 2})
    return m


@pytest.mark.parametrize("kind", ["seam", "orphan", "merge", "nothing"])
def test_erase_stages_match_command(monkeypatch, kind):
    m, ref = patch(kind), patch(kind)
    def targets(mesh):
        if kind == "seam":
            return [], [mesh.faces[0]]
        if kind == "orphan":
            return [mesh.edges[0]], []
        if kind == "merge":
            return [next(e for e in mesh.edges if len(e.faces)==2)], []
        return [], []
    es, fs = targets(m)
    rs, rf = targets(ref)
    command = EraseSelectionCommand(rs, rf)
    command._journal_geometry = True
    command.do(SimpleNamespace(mesh=ref, selection=set(), version=0))
    before = m._mut_serial
    gone_edges, gone_faces, planes, across, dirty, footprint = MeshStore.command_erase_selection(
        m, [[v.toTuple() for v in f.vertices] for f in fs], [(e.a.toTuple(),e.b.toTuple()) for e in es])
    assert not planes and not across
    assert all(isinstance(k,int) for k in gone_edges+gone_faces)
    assert dirty == m._mut_serial-before
    at_cleanup = m._mut_serial
    seams, healed, vertices, extra = MeshStore.command_erase_cleanup(m,footprint)
    assert seams == int(kind == "seam")
    assert vertices == (2 if kind == "orphan" else 0)
    assert all(isinstance(k,int) for k in healed)
    assert extra == m._mut_serial-at_cleanup
    assert m.revision == ref.revision
    assert fingerprint(m) == fingerprint(ref)



def test_erase_returns_plane_and_across_keys_for_rebuild(monkeypatch):
    m = Mesh()
    m.add_face([(0,0,0), (4,0,0), (4,4,0), (2,4,0), (2,2,0), (2,4,0), (0,4,0)])
    slit = [(2.,4.,0.), (2.,2.,0.)]
    key = m.find_edge(m.vertex_at(slit[0]), m.vertex_at(slit[1])).key
    edges, _, planes, across, _, footprint = MeshStore.command_erase_selection(m, [], [slit])
    assert edges == [key] and len(planes) == 1 and not across
    assert tuple(planes[0][0]) == (0.,0.,0.) and tuple(planes[0][1]) == (0.,0.,1.)
    assert footprint is not None

    m = patch("merge")
    a, b = (0.,1.,0.), (2.,1.,0.)
    wall = m.add_face([a,b,(2.,1.,2.),(0.,1.,2.)])
    _, _, planes, across, _, footprint = MeshStore.command_erase_selection(m, [], [(a,b)])
    assert not planes and across == [wall.key]
    assert footprint is None
