"""Direct and mixed command replay must preserve materialised element handles."""
from ingetrazo_core import Vec3
from core.history import (AddEdgeCommand, AddFaceCommand, CompoundCommand,
                          DeleteEdgesCommand, DeleteFaceCommand, EraseSelectionCommand,
                          ExplodeCurvesCommand, History, SetNorthCommand)
from core.scene import Scene
from core.store_mesh import StoreMesh


def materialised_scene():
    scene = Scene()
    if isinstance(scene.mesh, StoreMesh):
        scene.mesh._materialise()
    return scene


def ring(low, high):
    return [Vec3(x, y, 0) for x, y in
            [(low, low), (high, low), (high, high), (low, high)]]


def test_direct_draw_delete_and_snapshot_replay_keep_live_handles():
    scene = materialised_scene()
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
            assert len(before) == len(after) and all(a is b for a, b in zip(before, after))
        assert child.attrs["marker"] == "kept"
        assert child in edge.faces and mother in edge.faces
    draw.undo(scene)
    assert mesh.faces == [mother] and not mother.holes
    draw.do(scene)
    assert draw.face is child and mesh.faces[1] is child


def test_mixed_curve_and_host_replay_keeps_direct_edge_identity():
    scene = materialised_scene()
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
    assert edge is add.edge and edge.v1 is vertex
    history = History(scene)
    command = CompoundCommand([ExplodeCurvesCommand([first, edge]), SetNorthCommand(31)])
    history.execute(command)
    assert not history.last_error and first.curve is None and edge.curve is None
    assert scene.north_deg == 31
    assert history.undo() and first.curve == edge.curve == token and scene.north_deg == 0
    assert history.redo() and first.curve is None and edge.curve is None
