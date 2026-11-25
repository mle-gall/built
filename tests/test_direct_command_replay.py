"""Direct and mixed command replay preserve store handles and geometry ownership."""
import pytest

from built_core import Matrix4, Vec3
from core.group import Group
from core.mesh import Mesh
from core.history import (AddEdgeCommand, AddFaceCommand, Command, CommandRejected, CompoundCommand,
                          DeleteEdgesCommand, DeleteFaceCommand, EraseSelectionCommand,
                          ExplodeCurvesCommand, History, SetNorthCommand)
from core.scene import Scene


def ring(low, high):
    return [Vec3(x, y, 0) for x, y in
            [(low, low), (high, low), (high, high), (low, high)]]


def test_direct_draw_delete_and_snapshot_replay_keep_live_handles():
    scene = Scene()
    mesh = scene.mesh
    mother = mesh.add_face(ring(0, 5))
    mother.attrs["color"] = [1, 0, 0]
    draw = AddFaceCommand(ring(1, 3))
    draw.do(scene)
    child = draw.face
    assert len(mother.holes) == 1 and child.attrs["color"] == mother.attrs["color"]
    edge = mesh.find_edge(child.loop[0], child.loop[1])
    handles = (list(mesh.vertices), list(mesh.edges), list(mesh.faces))
    child.attrs["marker"] = "kept"
    for command in (DeleteFaceCommand(child), DeleteEdgesCommand([edge]),
                    EraseSelectionCommand([edge], [child])):
        command.do(scene)
        command.undo(scene)
        for before, after in zip(handles, (mesh.vertices, mesh.edges, mesh.faces)):
            assert len(before) == len(after) and [a.key for a in before] == [b.key for b in after]
        assert child.attrs["marker"] == "kept"
        assert child in edge.faces and mother in edge.faces
    draw.undo(scene)
    assert mesh.faces == [mother] and not mother.holes
    draw.do(scene)
    assert draw.face == child and mesh.faces[1] == child


def test_mixed_curve_and_host_replay_keeps_direct_edge_identity():
    scene = Scene()
    mesh = scene.mesh
    a, b, c = Vec3(), Vec3(1, 0, 0), Vec3(2, 0, 0)
    first = mesh.add_edge(a, b)
    token = 2 ** 96
    first.curve = token
    add = AddEdgeCommand(b, c, soft=True, curve=token)
    add.do(scene)
    edge, vertex = add.edge, add.edge.v1
    add.undo(scene)
    assert mesh.edges == [first] and vertex not in mesh.vertices
    add.do(scene)
    assert edge == add.edge and edge.v1 == vertex
    history = History(scene)
    command = CompoundCommand([ExplodeCurvesCommand([first, edge]), SetNorthCommand(31)])
    history.execute(command)
    assert not history.last_error and first.curve is None and edge.curve is None
    assert scene.north_deg == 31
    assert history.undo() and first.curve == edge.curve == token and scene.north_deg == 0
    assert history.redo() and first.curve is None and edge.curve is None


@pytest.mark.parametrize("occurrence", [False, True])
def test_rejected_mixed_edit_restores_each_geometry_owner(occurrence):
    scene = Scene()
    loose = scene.mesh
    if occurrence:
        group = Group(Mesh())
        group.xform = Matrix4()
        group.xform.translate(8, 2, 0)
        scene.groups.append(group)
        scene.begin_group_edit(group)

    mesh = scene.mesh
    existing = mesh.add_edge(Vec3(), Vec3(1, 0, 0))
    existing.curve = 17
    history = History(scene)
    old_north = scene.north_deg

    class Reject(Command):
        def do(self, scene):
            assert existing.curve is None and scene.north_deg == 37
            raise CommandRejected("declined")

        def undo(self, scene):
            pytest.fail("attempted to replay a rejected child")

    history.execute(CompoundCommand([ExplodeCurvesCommand([existing]),
                                     SetNorthCommand(37), Reject()]))
    assert history.last_rejected == "declined"
    assert history.revision == 0 and not history.undo_stack
    assert scene.north_deg == old_north and existing.curve == 17
    assert [e.key for e in mesh.edges] == [existing.key]
    assert mesh.edges[0] == existing
    if occurrence:
        scene.end_group_edit()
        assert scene.mesh is loose and not loose.edges
        assert [edge.curve for edge in group.mesh.edges] == [17]

def test_plugin_reordering_keeps_the_store_and_held_edges():
    from built_core.store_mesh import StoreMesh
    scene = Scene()
    mesh = scene.mesh
    mesh.add_face(ring(0, 2))
    held = list(mesh.edges)
    mesh.edges.reverse()
    assert list(mesh.edges) == held[::-1]
    mesh.edges.sort(key=lambda edge: edge.key)
    assert list(mesh.edges) == sorted(held, key=lambda edge: edge.key)
    mesh.edges[:] = held[::-1]
    mesh.edges = held
    assert type(mesh) is StoreMesh and list(mesh.edges) == held
    cmd = AddEdgeCommand(Vec3(3, 0, 0), Vec3(4, 0, 0))
    cmd.do(scene)
    assert mesh.edges[-1] == cmd.edge
    cmd.undo(scene)
    assert list(mesh.edges) == held
