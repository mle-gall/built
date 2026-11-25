"""Command entry points return keys and enlist the same geometry as the scripting facade."""
from types import SimpleNamespace

import pytest
from ingetrazo_core import Matrix4, MeshStore, Vec3

from core.history import AddEdgeCommand, History, SnapshotMutation
from core.layers import DEFAULT_LAYER
from core.mesh import Mesh
from core.scene import Scene
from core import store_mesh


def test_draw_edge_matches_command_on_creation_and_restamps(monkeypatch):
    monkeypatch.setenv("INGETRAZO_STORE", "1")
    mesh, reference = Mesh(), Mesh()
    a, b = (4., 0., 1.), (5., 0., 1.)
    for soft, curve, layer, owned, changed in (
        (False, 12, "outline", True, False),
        (None, None, None, False, False),
        (True, -(2**63), DEFAULT_LAYER, False, True),
        (True, 2**63-1, "section", False, True),
        (None, None, "section", False, True),
    ):
        command = AddEdgeCommand(Vec3(*a), Vec3(*b), soft=soft, curve=curve, layer=layer)
        command._journal_geometry = True
        command.do(SimpleNamespace(mesh=reference, version=0))
        key, flags, dirty = MeshStore.command_draw_edge(mesh, a, b, soft, curve, layer)
        assert (key is not None, flags, dirty) == (owned, changed, int(owned))
        assert command._owned == owned
        assert len(mesh.edges) == len(reference.edges) == 1
        edge, expected = mesh.edges[0], reference.edges[0]
        if owned:
            assert edge.key == key
        assert (edge.soft, edge.curve, edge.layer) == (expected.soft, expected.curve, expected.layer)
        assert (mesh.revision, mesh._mut_serial) == (reference.revision, reference._mut_serial)
    with pytest.raises(OverflowError):
        MeshStore.command_draw_edge(mesh, a, b, curve=2**63)


def test_draw_edge_refusal_precedes_mutation(monkeypatch):
    a, b = (0., 0., 0.), (1., 0., 0.)
    monkeypatch.setenv("INGETRAZO_STORE", "0")
    detached = Mesh()
    assert MeshStore.command_draw_edge(detached, a, b) is None
    assert not detached.edges
    monkeypatch.setenv("INGETRAZO_STORE", "1")
    mesh = Mesh()
    mesh._planned_points = {}
    assert MeshStore.command_draw_edge(mesh, a, b) is None
    del mesh._planned_points
    monkeypatch.setattr(store_mesh, "_hypot", lambda: None)
    assert MeshStore.command_draw_edge(mesh, a, b) is None
    assert not mesh.vertices and mesh._mut_serial == 0


def test_draw_edge_in_occurrence_restores_definition_key(monkeypatch):
    monkeypatch.setenv("INGETRAZO_STORE", "1")
    scene = Scene()
    definition = scene.mesh
    frame = Matrix4()
    frame.translate(10., 0., 0.)
    occurrence = store_mesh.instance_view(definition, frame.data())
    history = History(scene)
    results = []
    def draw(_):
        results.append(MeshStore.command_draw_edge(occurrence, (10., 1., 0.), (12., 1., 0.)))
    history.execute(SnapshotMutation(draw, mesh=occurrence))
    key, = [e.key for e in definition.edges]
    assert results == [(key, False, 1)]
    assert definition.edges[0].a.toTuple() == (0., 1., 0.)
    assert occurrence.edges[0].a.toTuple() == (10., 1., 0.)
    assert history.undo() and not definition.edges
    assert history.redo() and definition.edges[0].key == occurrence.edges[0].key == key
