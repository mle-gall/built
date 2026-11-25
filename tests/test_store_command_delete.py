"""Keyed removal has the same ordered incidence and dirty state as History."""
from types import SimpleNamespace

import pytest
from built_core import MeshStore
from built_core.history import DeleteEdgesCommand, DeleteFaceCommand
from built_core.mesh import Mesh
from tests.test_store_mesh import fingerprint


def graph():
    m = Mesh()
    for x in (0., 1., 650_000.):
        m.add_face([(x,0,0), (x+1,0,0), (x+1,1,0), (x,1,0)])
    return m


@pytest.mark.parametrize("scan", ["0", "1"])
@pytest.mark.parametrize("cascade", [False, True])
def test_delete_edges_returns_command_keys_in_order(monkeypatch, scan, cascade):
    monkeypatch.setenv("BUILT_SCANS", scan)
    m, ref = graph(), graph()
    first = [0,4,len(m.edges)-1]
    targets = [m.edges[i] for i in first]
    command = DeleteEdgesCommand([ref.edges[i] for i in first], cascade_faces=cascade)
    start = m._mut_serial
    edge_order = [e.key for e in m.edges]
    face_order = [f.key for f in m.faces]
    ref_edges = [e.key for e in ref.edges]
    ref_faces = [f.key for f in ref.faces]
    command._journal_geometry = True
    command.do(SimpleNamespace(mesh=ref, selection=set(), version=0))
    edges, faces, dirty = MeshStore.command_delete_edges(m, [(e.a.toTuple(),e.b.toTuple()) for e in targets], cascade)
    assert edges == [edge_order[ref_edges.index(e.key)] for e in command.removed_edges]
    assert faces == [face_order[ref_faces.index(f.key)] for f in command.removed_faces]
    assert dirty == m._mut_serial - start == len(edges)+len(faces)
    assert fingerprint(m) == fingerprint(ref)
    assert m.revision == ref.revision


@pytest.mark.parametrize("scan", ["0", "1"])
def test_delete_face_distinguishes_missing_from_refusal(monkeypatch, scan):
    monkeypatch.setenv("BUILT_SCANS", scan)
    m, ref = graph(), graph()
    key = m.faces[-1].key
    outer = [p.toTuple() for p in reversed(m.faces[-1].vertices)]
    command = DeleteFaceCommand(ref.faces[-1])
    command._journal_geometry = True
    command.do(SimpleNamespace(mesh=ref, selection=set(), version=0))
    assert MeshStore.command_delete_face(m, outer) == (key, 1)
    assert fingerprint(m) == fingerprint(ref)
    after = m.revision
    assert MeshStore.command_delete_face(m, outer) == (None, 0)
    assert m.revision == after
